use crate::chat::ask::{yes_no, Ask};
use crate::chat::session::{Emit, Event};
use crate::schedule_store::ScheduleStore;
use crate::warn::warn_if_err;
use crate::watches::Watches;
use crate::worker_mcp::Inbox;
use crate::workers::Workers;
use ralph_core::schedule::{
    approval_question, plan_restore, plan_tick, proposed, to_watch_spec, Task, TaskStatus,
};
use ralph_core::schedule_command::{
    cancel_message, list_tasks, plan_cancel, CancelPlan, CancelStop, ScheduleCommand,
    SCHEDULE_USAGE,
};
use ralph_core::schedule_resume::{plan_lost_turns, LostTurn};
use ralph_core::schedule_worker::worker_id;
use ralph_core::watch_request::start_failure_report;
use ralph_core::worker::{gone_path, Worker};
use std::fs;
use std::path::Path;
use std::time::Duration;

pub fn restore_schedule(
    store: &ScheduleStore,
    store_dir: &Path,
    workers: &[Worker],
) -> Vec<LostTurn> {
    let tasks = store.load_all();
    let now = chrono::Utc::now();
    let (restored, lost) = plan_lost_turns(plan_restore(tasks.clone(), now, workers), workers, now);
    for task in restored
        .into_iter()
        .filter(|restored| !tasks.contains(restored))
    {
        warn_if_err(
            store.save(&task),
            format!("restoring scheduled task {}", task.id),
        );
    }
    for turn in &lost {
        fs::remove_file(gone_path(store_dir, &worker_id(&turn.task))).ok();
    }
    lost
}

pub fn fire_due(store: &ScheduleStore, watches: &Watches, inbox: &Inbox) {
    let tasks = store.load_all();
    if tasks.is_empty() {
        return;
    }
    let active: Vec<String> = watches.list().into_iter().map(|watch| watch.id).collect();
    if watches.is_closing() {
        return;
    }
    let now = chrono::Utc::now();
    let offset = *chrono::Local::now().offset();
    let plan = plan_tick(&tasks, &active, now);
    for task in plan.finished {
        crate::schedule_rearm::finish_task(store, task, now);
    }
    for task in plan.fire {
        let Some(spec) = to_watch_spec(task, now, offset) else {
            continue;
        };
        match watches.start(spec) {
            Ok(_) => warn_if_err(
                store.save(&Task {
                    status: TaskStatus::Running,
                    ..task.clone()
                }),
                format!("marking scheduled task {} running", task.id),
            ),
            Err(error) => {
                let report = start_failure_report(
                    &ralph_core::schedule::watch_id(&task.id),
                    &error.to_string(),
                    &now.to_rfc3339(),
                );
                warn_if_err(
                    inbox.post(&report, 0),
                    format!("posting schedule {}", task.id),
                );
                store.remove(&task.id);
            }
        }
    }
}

pub fn approve_proposed(
    store: &ScheduleStore,
    default_timeout: Duration,
    ask: &mut dyn Ask,
    emit: &Emit,
) -> bool {
    let tasks = proposed(store.load_all());
    for task in &tasks {
        if decide(task, default_timeout, ask) {
            let armed = store.save(&Task {
                status: TaskStatus::Pending,
                ..task.clone()
            });
            match armed {
                Ok(()) => emit(Event::Notice(format!("Armed {}.", task.id))),
                Err(error) => emit(Event::Notice(format!("Could not arm {}: {error}", task.id))),
            }
        } else {
            store.remove(&task.id);
            emit(Event::Notice(format!("Cancelled {}.", task.id)));
        }
    }
    !tasks.is_empty()
}

fn decide(task: &Task, default_timeout: Duration, ask: &mut dyn Ask) -> bool {
    let question = || {
        let now = chrono::Utc::now();
        let offset = *chrono::Local::now().offset();
        approval_question(task, now, offset, default_timeout)
    };
    yes_no(ask, question).unwrap_or(false)
}

pub fn run(
    command: ScheduleCommand,
    store: &ScheduleStore,
    watches: &Watches,
    workers: &Workers,
) -> String {
    match command {
        ScheduleCommand::List => list_tasks(
            &store.load_all(),
            chrono::Utc::now(),
            *chrono::Local::now().offset(),
        ),
        ScheduleCommand::Cancel(id) => {
            let plan = plan_cancel(&id, &store.load_all());
            if let CancelPlan::Remove { id, stop } = &plan {
                store.remove(id);
                match stop {
                    Some(CancelStop::Watch(watch)) => {
                        watches.stop(watch);
                    }
                    Some(CancelStop::Worker(worker)) => warn_if_err(
                        workers.stop_harness(worker),
                        format!("stopping cancelled worker {worker}"),
                    ),
                    None => {}
                }
            }
            cancel_message(&plan)
        }
        ScheduleCommand::Usage => SCHEDULE_USAGE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ralph_core::schedule::TaskKind;
    use ralph_core::worker::{HarnessId, SessionRef, WorkerId};
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct Scripted {
        queue: std::vec::IntoIter<Option<String>>,
        prompts: String,
    }

    impl Ask for Scripted {
        fn answer(&mut self, prompt: &str) -> Option<String> {
            self.prompts.push_str(prompt);
            self.queue.next().flatten()
        }

        fn edit(&mut self, _path: &Path) -> io::Result<()> {
            Ok(())
        }
    }

    fn run(store: &ScheduleStore, answers: &[Option<&str>]) -> (bool, String, String) {
        let mut ask = Scripted {
            queue: answers
                .iter()
                .map(|a| a.map(str::to_string))
                .collect::<Vec<_>>()
                .into_iter(),
            prompts: String::new(),
        };
        let lines = Arc::new(Mutex::new(String::new()));
        let sink = lines.clone();
        let emit: Emit = Arc::new(move |event| {
            if let (Ok(mut lines), Event::Notice(line)) = (sink.lock(), event) {
                lines.push_str(&line);
                lines.push('\n');
            }
        });
        let asked = approve_proposed(store, Duration::from_secs(60), &mut ask, &emit);
        let notices = lines.lock().map(|lines| lines.clone()).unwrap_or_default();
        (asked, ask.prompts, notices)
    }

    fn propose(store: &ScheduleStore, id: &str, minutes: i64) {
        store
            .save(&Task {
                id: id.to_string(),
                kind: TaskKind::Command {
                    command: format!("echo {id}"),
                },
                follow_up: None,
                at: Utc::now() + chrono::Duration::minutes(minutes),
                timeout_secs: None,
                status: TaskStatus::Proposed,
                late: false,
                deadline: None,
                repeat: None,
            })
            .unwrap();
    }

    fn statuses(store: &ScheduleStore) -> Vec<(String, TaskStatus)> {
        let mut tasks: Vec<_> = store
            .load_all()
            .into_iter()
            .map(|t| (t.id, t.status))
            .collect();
        tasks.sort_by(|a, b| a.0.cmp(&b.0));
        tasks
    }

    #[test]
    fn restore_requeues_running_and_marks_missed_tasks_late() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "running", -10);
        propose(&store, "missed", -5);
        propose(&store, "future", 30);
        propose(&store, "unasked", -1);
        for (id, status) in [
            ("running", TaskStatus::Running),
            ("missed", TaskStatus::Pending),
            ("future", TaskStatus::Pending),
        ] {
            let mut task = store.load_all().into_iter().find(|t| t.id == id).unwrap();
            task.status = status;
            store.save(&task).unwrap();
        }
        restore_schedule(&store, dir.path(), &[]);
        let mut view: Vec<(String, TaskStatus, bool)> = store
            .load_all()
            .into_iter()
            .map(|t| (t.id, t.status, t.late))
            .collect();
        view.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            view,
            vec![
                ("future".to_string(), TaskStatus::Pending, false),
                ("missed".to_string(), TaskStatus::Pending, true),
                ("running".to_string(), TaskStatus::Pending, true),
                ("unasked".to_string(), TaskStatus::Proposed, false),
            ]
        );
    }

    fn mark_running(store: &ScheduleStore, id: &str) {
        let mut task = store.load_all().into_iter().find(|t| t.id == id).unwrap();
        task.status = TaskStatus::Running;
        store.save(&task).unwrap();
    }

    #[test]
    fn a_restore_drops_the_old_exit_mark_of_a_lost_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("schedule"));
        propose(&store, "a", -1);
        let mut task = store.load_all().into_iter().next().unwrap();
        task.kind = TaskKind::Worker {
            brief: "do it".to_string(),
        };
        task.status = TaskStatus::Running;
        store.save(&task).unwrap();
        let saved = Worker::saved(
            WorkerId::parse("sched-a").unwrap(),
            HarnessId::Claude,
            SessionRef::new(dir.path().join("claude-session/sched-a")),
        )
        .unwrap();
        let marks = ["sched-a", "sched-b"].map(|id| gone_path(dir.path(), id));
        for mark in &marks {
            std::fs::create_dir_all(mark.parent().unwrap()).unwrap();
            std::fs::write(mark, "claude exited").unwrap();
        }
        let lost = restore_schedule(&store, dir.path(), &[saved]);
        assert_eq!(lost.len(), 1);
        assert_eq!(marks.map(|mark| mark.exists()), [false, true]);
    }

    #[test]
    fn a_running_task_survives_a_tick_during_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("schedule"));
        let inbox = std::sync::Arc::new(Inbox::new(dir.path().join("inbox")));
        let watches = Watches::new(
            std::sync::Arc::clone(&inbox),
            dir.path(),
            Duration::from_secs(60),
        );
        propose(&store, "a", -1);
        mark_running(&store, "a");
        watches.kill_all();
        fire_due(&store, &watches, &inbox);
        assert_eq!(
            statuses(&store),
            vec![("a".to_string(), TaskStatus::Running)]
        );
        restore_schedule(&store, dir.path(), &[]);
        let restored = store.load_all();
        assert_eq!(restored[0].status, TaskStatus::Pending);
        assert!(restored[0].late);
    }

    #[test]
    fn a_running_task_without_its_watch_is_finished_while_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("schedule"));
        let inbox = std::sync::Arc::new(Inbox::new(dir.path().join("inbox")));
        let watches = Watches::new(
            std::sync::Arc::clone(&inbox),
            dir.path(),
            Duration::from_secs(60),
        );
        propose(&store, "a", -1);
        mark_running(&store, "a");
        fire_due(&store, &watches, &inbox);
        assert!(statuses(&store).is_empty());
    }

    #[test]
    fn yes_arms_and_no_cancels_in_time_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "first", 5);
        propose(&store, "second", 10);
        let (_, prompts, notices) = run(&store, &[Some("YES\n"), Some("n\n")]);
        assert_eq!(
            statuses(&store),
            vec![("first".to_string(), TaskStatus::Pending)]
        );
        assert_eq!(notices, "Armed first.\nCancelled second.\n");
        assert!(prompts.find("echo first").unwrap() < prompts.find("echo second").unwrap());
    }

    #[test]
    fn unclear_answers_repeat_the_same_question() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "a", 5);
        let (_, prompts, _) = run(&store, &[Some("go\n"), Some("\n"), Some("y\n")]);
        assert_eq!(prompts.matches("Arm it? [y/n]").count(), 3);
        assert_eq!(
            statuses(&store),
            vec![("a".to_string(), TaskStatus::Pending)]
        );
    }

    #[test]
    fn interrupt_or_end_of_input_cancels_the_task() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "a", 5);
        let (_, _, notices) = run(&store, &[None]);
        assert!(statuses(&store).is_empty());
        assert_eq!(notices, "Cancelled a.\n");
    }

    #[test]
    fn armed_and_running_tasks_are_not_asked_about() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "a", 5);
        assert!(run(&store, &[Some("y")]).0);
        let (asked, prompts, notices) = run(&store, &[]);
        assert!(!asked);
        assert!(prompts.is_empty() && notices.is_empty());
        assert_eq!(
            statuses(&store),
            vec![("a".to_string(), TaskStatus::Pending)]
        );
    }

    #[test]
    fn an_empty_store_asks_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        assert_eq!(run(&store, &[]), (false, String::new(), String::new()));
    }

    #[test]
    fn a_failed_save_is_reported_instead_of_armed() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        propose(&store, "a", 5);
        let tmp = dir
            .path()
            .join(format!(".{}", ralph_core::watch_store::file_name("a")));
        std::fs::create_dir(&tmp).unwrap();
        let (_, _, notices) = run(&store, &[Some("y")]);
        assert!(notices.contains("Could not arm a:"), "{notices}");
        assert!(!notices.contains("Armed a."), "{notices}");
        assert_eq!(
            statuses(&store),
            vec![("a".to_string(), TaskStatus::Proposed)]
        );
    }
}
