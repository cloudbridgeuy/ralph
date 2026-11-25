use crate::schedule::{Task, TaskStatus};
use crate::schedule_worker::{is_worker_task, worker_deadline, worker_id, STARTUP_GRACE_SECS};
use crate::worker::{branch_name, SessionRef, Worker, WorkerState};
use chrono::{DateTime, Duration, Utc};

pub const FINISH_TURN_MESSAGE: &str =
    "your turn was interrupted by a restart; check what you already did and finish";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    Unsent,
    Sent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LostTurn {
    pub task: String,
    pub since: DateTime<Utc>,
    pub message: Message,
    pub session: SessionRef,
    pub limit: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishStep {
    Send,
    Wait,
    Fail,
}

pub fn plan_lost_turns(
    tasks: Vec<Task>,
    workers: &[Worker],
    now: DateTime<Utc>,
) -> (Vec<Task>, Vec<LostTurn>) {
    let (tasks, lost): (Vec<_>, Vec<_>) = tasks
        .into_iter()
        .map(|task| match lost_turn(&task, workers, now) {
            Some(turn) => {
                let deadline = Some(worker_deadline(&task, now));
                (Task { deadline, ..task }, Some(turn))
            }
            None => (task, None),
        })
        .unzip();
    (tasks, lost.into_iter().flatten().collect())
}

fn lost_turn(task: &Task, workers: &[Worker], now: DateTime<Utc>) -> Option<LostTurn> {
    if !is_open_run(task) {
        return None;
    }
    let id = worker_id(&task.id);
    workers
        .iter()
        .filter(|worker| worker.id().as_str() == id)
        .find_map(|worker| match worker.state() {
            WorkerState::Saved(session) => Some(LostTurn {
                task: task.id.clone(),
                since: now,
                message: Message::Unsent,
                session: session.clone(),
                limit: worker.resume_limit(),
            }),
            WorkerState::Running => None,
        })
}

fn is_open_run(task: &Task) -> bool {
    is_worker_task(task) && task.status == TaskStatus::Running
}

pub fn finish_step(turn: &LostTurn, running: bool, now: DateTime<Utc>) -> FinishStep {
    match (turn.message, running) {
        (Message::Unsent, true) => FinishStep::Send,
        (Message::Sent, true) => FinishStep::Wait,
        (Message::Unsent, false) if now < turn.since + Duration::seconds(STARTUP_GRACE_SECS) => {
            FinishStep::Wait
        }
        (Message::Unsent | Message::Sent, false) => FinishStep::Fail,
    }
}

pub fn has_open_run(tasks: &[Task], worker: &str) -> bool {
    tasks
        .iter()
        .any(|task| is_open_run(task) && worker_id(&task.id) == worker)
}

pub fn resume_failed_text(turn: &LostTurn) -> String {
    let limit = turn
        .limit
        .map(|limit| format!(" Resume limit: {limit}."))
        .unwrap_or_default();
    format!(
        "Scheduled worker `{}` needs you: a restart interrupted its turn and the resumed session did not finish it. Saved session: {}.{limit} Worktree and branch {} are kept.",
        turn.task,
        turn.session.record().display(),
        branch_name(&worker_id(&turn.task)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::TaskKind;
    use crate::schedule_repeat::Repeat;
    use crate::worker::{HarnessId, WorkerId};
    use std::path::PathBuf;

    const RECORD: &str = "/store/claude-session/sched-a";

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn started() -> DateTime<Utc> {
        utc("2026-11-25T01:00:00Z")
    }

    fn restarted() -> DateTime<Utc> {
        utc("2026-11-25T09:00:00Z")
    }

    fn task(id: &str, status: TaskStatus) -> Task {
        Task {
            id: id.to_string(),
            kind: TaskKind::Worker {
                brief: "do it".to_string(),
            },
            follow_up: None,
            at: started(),
            timeout_secs: Some(3600),
            status,
            late: false,
            deadline: Some(started() + Duration::seconds(3600)),
            repeat: Some(Repeat::Every { every_secs: 86400 }),
        }
    }

    fn id() -> WorkerId {
        WorkerId::parse("sched-a").unwrap()
    }

    fn session() -> SessionRef {
        SessionRef::new(PathBuf::from(RECORD))
    }

    fn saved() -> Worker {
        Worker::saved(id(), HarnessId::Claude, session()).unwrap()
    }

    fn turn(message: Message) -> LostTurn {
        LostTurn {
            task: "a".to_string(),
            since: restarted(),
            message,
            session: session(),
            limit: Some("same cwd required"),
        }
    }

    #[test]
    fn a_running_task_with_a_saved_worker_has_a_lost_turn() {
        let (_, lost) = plan_lost_turns(
            vec![task("a", TaskStatus::Running)],
            &[Worker::running(id(), HarnessId::Claude), saved()],
            restarted(),
        );
        assert_eq!(
            lost,
            vec![LostTurn {
                limit: saved().resume_limit(),
                ..turn(Message::Unsent)
            }]
        );
    }

    #[test]
    fn a_lost_turn_gets_a_fresh_deadline_and_keeps_its_repeat_grid() {
        let before = task("a", TaskStatus::Running);
        let (tasks, _) = plan_lost_turns(vec![before.clone()], &[saved()], restarted());
        assert_eq!(
            tasks,
            vec![Task {
                deadline: Some(restarted() + Duration::seconds(3600)),
                ..before
            }]
        );
    }

    #[test]
    fn only_a_running_worker_task_with_a_saved_worker_has_a_lost_turn() {
        let command = Task {
            kind: TaskKind::Command {
                command: "true".to_string(),
            },
            ..task("a", TaskStatus::Running)
        };
        let running = Worker::running(id(), HarnessId::Claude);
        for (tasks, workers) in [
            (vec![task("a", TaskStatus::Running)], vec![running]),
            (vec![task("a", TaskStatus::Pending)], vec![saved()]),
            (vec![task("a", TaskStatus::Proposed)], vec![saved()]),
            (vec![task("b", TaskStatus::Running)], vec![saved()]),
            (vec![command], vec![saved()]),
        ] {
            let (after, lost) = plan_lost_turns(tasks.clone(), &workers, restarted());
            assert_eq!((after, lost), (tasks, Vec::new()));
        }
    }

    #[test]
    fn the_message_is_sent_once_and_a_stop_after_it_fails_the_turn() {
        let now = restarted();
        assert_eq!(
            FINISH_TURN_MESSAGE,
            "your turn was interrupted by a restart; check what you already did and finish"
        );
        assert_eq!(
            finish_step(&turn(Message::Unsent), true, now),
            FinishStep::Send
        );
        assert_eq!(
            finish_step(&turn(Message::Sent), true, now),
            FinishStep::Wait
        );
        assert_eq!(
            finish_step(&turn(Message::Sent), false, now),
            FinishStep::Fail
        );
    }

    #[test]
    fn a_worker_that_does_not_resume_in_time_fails_the_turn() {
        let limit = restarted() + Duration::seconds(STARTUP_GRACE_SECS);
        let early = limit - Duration::seconds(1);
        assert_eq!(
            finish_step(&turn(Message::Unsent), false, early),
            FinishStep::Wait
        );
        assert_eq!(
            finish_step(&turn(Message::Unsent), false, limit),
            FinishStep::Fail
        );
    }

    #[test]
    fn only_a_running_worker_task_is_an_open_run() {
        let running = [task("a", TaskStatus::Running)];
        assert!(has_open_run(&running, "sched-a"));
        assert!(!has_open_run(&running, "sched-b"));
        assert!(!has_open_run(&running, "a"));
        assert!(!has_open_run(&[task("a", TaskStatus::Pending)], "sched-a"));
        assert!(!has_open_run(&[], "sched-a"));
    }

    #[test]
    fn a_failed_resume_names_the_session_and_the_resume_limit() {
        let text = resume_failed_text(&turn(Message::Unsent));
        assert_eq!(
            text,
            format!(
                "Scheduled worker `a` needs you: a restart interrupted its turn and the resumed session did not finish it. Saved session: {RECORD}. Resume limit: same cwd required. Worktree and branch {} are kept.",
                branch_name("sched-a")
            )
        );
        let unlimited = resume_failed_text(&LostTurn {
            limit: None,
            ..turn(Message::Sent)
        });
        assert!(!unlimited.contains("Resume limit"), "{unlimited}");
    }
}
