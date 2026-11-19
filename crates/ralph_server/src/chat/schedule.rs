use crate::schedule_store::ScheduleStore;
use crate::warn::warn_if_err;
use crate::watches::Watches;
use crate::worker_mcp::Inbox;
use ralph_core::schedule::{plan_restore, plan_tick, to_watch_spec, Task, TaskStatus};
use ralph_core::watch_request::start_failure_report;

pub fn restore_schedule(store: &ScheduleStore, alive_workers: &[String]) {
    let tasks = store.load_all();
    for task in plan_restore(tasks.clone(), chrono::Utc::now(), alive_workers)
        .into_iter()
        .filter(|restored| !tasks.contains(restored))
    {
        warn_if_err(
            store.save(&task),
            format!("restoring scheduled task {}", task.id),
        );
    }
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
        store.remove(&task.id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ralph_core::schedule::TaskKind;
    use std::time::Duration;

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
        restore_schedule(&store, &[]);
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
        restore_schedule(&store, &[]);
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
}
