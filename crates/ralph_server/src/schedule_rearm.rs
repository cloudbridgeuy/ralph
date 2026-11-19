use crate::schedule_store::ScheduleStore;
use crate::warn::warn_if_err;
use crate::worker_mcp::Inbox;
use chrono::{DateTime, Local, Utc};
use ralph_core::schedule::Task;
use ralph_core::schedule_repeat::{plan_rearm, plan_worker_rearm, repeat_stopped_text};
use ralph_core::schedule_worker::{chat_report, WorkerEnd};

fn settle(store: &ScheduleStore, task: &Task, next: Option<Task>) {
    match next {
        Some(next) => {
            let saved = store.save(&next);
            if saved.is_err() {
                store.remove(&task.id);
            }
            warn_if_err(
                saved,
                format!("repeat stopped: re-arming scheduled task {}", task.id),
            );
        }
        None => store.remove(&task.id),
    }
}

pub fn finish_task(store: &ScheduleStore, task: &Task, now: DateTime<Utc>) {
    settle(store, task, plan_rearm(task, now, &Local));
}

pub fn finish_worker(
    store: &ScheduleStore,
    inbox: &Inbox,
    task: &Task,
    end: &WorkerEnd,
    now: DateTime<Utc>,
) {
    if let Some(text) = repeat_stopped_text(task, end) {
        let report = chat_report(&task.id, text, &now.to_rfc3339());
        warn_if_err(
            inbox.post(&report, 0),
            format!("posting schedule {}", task.id),
        );
    }
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

    fn inbox_in(dir: &tempfile::TempDir) -> Inbox {
        Inbox::new(dir.path().join("inbox"))
    }

    fn posted_workers(dir: &tempfile::TempDir) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(dir.path().join("inbox")) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| std::fs::read(entry.ok()?.path()).ok())
            .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .filter_map(|value| value["worker"].as_str().map(str::to_string))
            .collect()
    }

    #[test]
    fn a_failed_rearm_save_removes_the_row() {
        let t = task(command(), hourly());
        let (dir, store) = saved(&t);
        let name = ralph_core::watch_store::file_name(&t.id);
        std::fs::create_dir(dir.path().join(format!(".{name}"))).unwrap();
        finish_task(&store, &t, Utc::now());
        assert!(store.load_all().is_empty());
        assert!(!dir.path().join(name).exists());
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
        let (dir, store) = saved(&t);
        let inbox = inbox_in(&dir);
        let now = Utc::now();
        finish_worker(&store, &inbox, &t, &WorkerEnd::Done, now);
        assert_rearmed(&store, now);
        assert!(posted_workers(&dir).is_empty());
    }

    #[test]
    fn a_repeating_worker_needing_attention_is_removed() {
        let t = task(worker(), hourly());
        let (dir, store) = saved(&t);
        let inbox = inbox_in(&dir);
        finish_worker(&store, &inbox, &t, &WorkerEnd::Attention, Utc::now());
        assert!(store.load_all().is_empty());
        assert_eq!(posted_workers(&dir), vec!["schedule:t".to_string()]);
    }

    #[test]
    fn a_one_shot_worker_is_removed() {
        let t = task(worker(), None);
        let (dir, store) = saved(&t);
        let inbox = inbox_in(&dir);
        finish_worker(&store, &inbox, &t, &WorkerEnd::Done, Utc::now());
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
