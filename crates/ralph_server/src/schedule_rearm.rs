use crate::schedule_store::ScheduleStore;
use crate::warn::warn_if_err;
use chrono::{DateTime, Local, Utc};
use ralph_core::schedule::Task;
use ralph_core::schedule_repeat::{plan_rearm, plan_worker_rearm};
use ralph_core::schedule_worker::WorkerEnd;

fn settle(store: &ScheduleStore, task: &Task, next: Option<Task>) {
    match next {
        Some(next) => warn_if_err(
            store.save(&next),
            format!("re-arming scheduled task {}", task.id),
        ),
        None => store.remove(&task.id),
    }
}

pub fn finish_task(store: &ScheduleStore, task: &Task, now: DateTime<Utc>) {
    settle(store, task, plan_rearm(task, now, &Local));
}

pub fn finish_worker(store: &ScheduleStore, task: &Task, end: &WorkerEnd, now: DateTime<Utc>) {
    settle(store, task, plan_worker_rearm(task, end, now, &Local));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::schedule::{TaskKind, TaskStatus};
    use ralph_core::schedule_repeat::Repeat;

    fn task(kind: TaskKind, repeat: Option<Repeat>) -> Task {
        Task {
            id: "t".to_string(),
            kind,
            follow_up: None,
            at: Utc::now() - chrono::Duration::minutes(1),
            timeout_secs: None,
            status: TaskStatus::Running,
            late: true,
            deadline: Some(Utc::now()),
            repeat,
        }
    }

    fn command() -> TaskKind {
        TaskKind::Command {
            command: "echo".to_string(),
        }
    }

    fn worker() -> TaskKind {
        TaskKind::Worker {
            brief: "b".to_string(),
        }
    }

    fn hourly() -> Option<Repeat> {
        Some(Repeat::Every { every_secs: 3600 })
    }

    fn saved(task: &Task) -> (tempfile::TempDir, ScheduleStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        store.save(task).unwrap();
        (dir, store)
    }

    fn assert_rearmed(store: &ScheduleStore, now: DateTime<Utc>) {
        let rows = store.load_all();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, TaskStatus::Pending);
        assert!(rows[0].at > now);
        assert!(!rows[0].late);
        assert_eq!(rows[0].deadline, None);
    }

    #[test]
    fn a_one_shot_command_is_removed() {
        let t = task(command(), None);
        let (_dir, store) = saved(&t);
        finish_task(&store, &t, Utc::now());
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn a_repeating_command_is_rearmed() {
        let t = task(command(), hourly());
        let (_dir, store) = saved(&t);
        let now = Utc::now();
        finish_task(&store, &t, now);
        assert_rearmed(&store, now);
    }

    #[test]
    fn a_repeating_worker_is_rearmed_when_done() {
        let t = task(worker(), hourly());
        let (_dir, store) = saved(&t);
        let now = Utc::now();
        finish_worker(&store, &t, &WorkerEnd::Done, now);
        assert_rearmed(&store, now);
    }

    #[test]
    fn a_repeating_worker_needing_attention_is_removed() {
        let t = task(worker(), hourly());
        let (_dir, store) = saved(&t);
        finish_worker(&store, &t, &WorkerEnd::Attention, Utc::now());
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn a_one_shot_worker_is_removed() {
        let t = task(worker(), None);
        let (_dir, store) = saved(&t);
        finish_worker(&store, &t, &WorkerEnd::Done, Utc::now());
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn fire_due_rearms_a_finished_repeating_command() {
        use crate::chat::schedule::fire_due;
        use crate::watches::Watches;
        use crate::worker_mcp::Inbox;
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("schedule"));
        let inbox = std::sync::Arc::new(Inbox::new(dir.path().join("inbox")));
        let watches = Watches::new(
            std::sync::Arc::clone(&inbox),
            dir.path(),
            std::time::Duration::from_secs(60),
        );
        store.save(&task(command(), hourly())).unwrap();
        fire_due(&store, &watches, &inbox);
        assert_rearmed(&store, Utc::now());
    }
}
