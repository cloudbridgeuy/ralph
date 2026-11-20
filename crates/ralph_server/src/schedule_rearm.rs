use crate::git_probe::branch_open;
use crate::schedule_store::ScheduleStore;
use crate::warn::warn_if_err;
use crate::worker_mcp::Inbox;
use crate::workers::repo_root;
use chrono::{DateTime, Local, Utc};
use ralph_core::schedule::{Task, TaskStatus};
use ralph_core::schedule_repeat::{
    open_branch_decision, plan_rearm, plan_worker_rearm, repeat_stopped_text, OpenBranch,
};
use ralph_core::schedule_worker::{chat_report, worker_id, WorkerEnd};
use ralph_core::worker::branch_name;

#[derive(Debug, PartialEq, Eq)]
enum Settle {
    Skip,
    Remove,
    Save(Task),
}

fn settle_plan(current: Option<&Task>, next: Option<Task>) -> Settle {
    match (current, next) {
        (Some(current), Some(next)) if current.status == TaskStatus::Running => Settle::Save(next),
        (Some(current), None) if current.status == TaskStatus::Running => Settle::Remove,
        _ => Settle::Skip,
    }
}

fn settle(store: &ScheduleStore, task: &Task, next: Option<Task>) -> bool {
    let current = store.load_all().into_iter().find(|row| row.id == task.id);
    match settle_plan(current.as_ref(), next) {
        Settle::Skip => false,
        Settle::Remove => {
            store.remove(&task.id);
            true
        }
        Settle::Save(next) => {
            let saved = store.save(&next);
            if saved.is_err() {
                store.remove(&task.id);
            }
            warn_if_err(
                saved,
                format!("repeat stopped: re-arming scheduled task {}", task.id),
            );
            true
        }
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
    let acted = settle(store, task, plan_worker_rearm(task, end, now, &Local));
    if !acted {
        return;
    }
    if let Some(text) = repeat_stopped_text(task, end) {
        let report = chat_report(&task.id, text, &now.to_rfc3339());
        warn_if_err(
            inbox.post(&report, 0),
            format!("posting schedule {}", task.id),
        );
    }
}

fn stop_if_open(
    is_open: impl Fn(&str) -> bool,
    store: &ScheduleStore,
    inbox: &Inbox,
    task: &Task,
    now: DateTime<Utc>,
) -> bool {
    if open_branch_decision(task, true) == OpenBranch::Start {
        return false;
    }
    let branch = branch_name(&worker_id(&task.id));
    match open_branch_decision(task, is_open(&branch)) {
        OpenBranch::Start => false,
        OpenBranch::Stop(text) => {
            let report = chat_report(&task.id, text, &now.to_rfc3339());
            warn_if_err(
                inbox.post(&report, 0),
                format!("posting schedule {}", task.id),
            );
            store.remove(&task.id);
            true
        }
    }
}

pub fn refuse_open_branch(
    monitor: &crate::chat::monitor::Monitor,
    task: &Task,
    now: DateTime<Utc>,
) -> bool {
    match repo_root() {
        Ok(repo) => stop_if_open(
            |branch| branch_open(&repo, branch),
            &monitor.schedule,
            &monitor.inbox,
            task,
            now,
        ),
        Err(error) => {
            crate::warn::warn(format!(
                "locating the repo for schedule {}: {error}",
                task.id
            ));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_probe::test_support::{branch_open_scrubbed, git};
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

    fn with_status(mut task: Task, status: TaskStatus) -> Task {
        task.status = status;
        task
    }

    #[test]
    fn settle_plan_skips_a_row_that_is_gone() {
        let next = with_status(task(command(), hourly()), TaskStatus::Pending);
        assert_eq!(settle_plan(None, Some(next)), Settle::Skip);
        assert_eq!(settle_plan(None, None), Settle::Skip);
    }

    #[test]
    fn settle_plan_skips_a_row_that_is_no_longer_running() {
        for status in [TaskStatus::Pending, TaskStatus::Proposed] {
            let current = with_status(task(command(), hourly()), status);
            let next = with_status(task(command(), hourly()), TaskStatus::Pending);
            assert_eq!(settle_plan(Some(&current), Some(next)), Settle::Skip);
            assert_eq!(settle_plan(Some(&current), None), Settle::Skip);
        }
    }

    #[test]
    fn settle_plan_removes_a_running_row_with_no_next_run() {
        let current = task(command(), None);
        assert_eq!(settle_plan(Some(&current), None), Settle::Remove);
    }

    #[test]
    fn settle_plan_saves_the_next_run_of_a_running_row() {
        let current = task(command(), hourly());
        let next = with_status(task(command(), hourly()), TaskStatus::Pending);
        assert_eq!(
            settle_plan(Some(&current), Some(next.clone())),
            Settle::Save(next)
        );
    }

    #[test]
    fn a_cancelled_repeating_task_is_not_revived() {
        let t = task(command(), hourly());
        let (dir, store) = saved(&t);
        store.remove(&t.id);
        finish_task(&store, &t, Utc::now());
        assert!(store.load_all().is_empty());
        assert!(!dir
            .path()
            .join(ralph_core::watch_store::file_name(&t.id))
            .exists());
    }

    #[test]
    fn a_cancelled_repeating_worker_is_not_revived() {
        let t = task(worker(), hourly());
        let (dir, store) = saved(&t);
        let inbox = inbox_in(&dir);
        store.remove(&t.id);
        finish_worker(&store, &inbox, &t, &WorkerEnd::Done, Utc::now());
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn a_row_proposed_again_under_the_same_id_is_left_alone() {
        let t = task(command(), hourly());
        let proposed = with_status(task(command(), None), TaskStatus::Proposed);
        let (_dir, store) = saved(&proposed);
        finish_task(&store, &t, Utc::now());
        assert_eq!(store.load_all(), vec![proposed]);
    }

    #[test]
    fn a_pending_row_is_not_overwritten() {
        let t = task(command(), hourly());
        let pending = with_status(task(command(), hourly()), TaskStatus::Pending);
        let (_dir, store) = saved(&pending);
        finish_task(&store, &t, Utc::now());
        assert_eq!(store.load_all(), vec![pending]);
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
    fn a_repeating_worker_no_longer_running_needing_attention_posts_no_notice() {
        let t = task(worker(), hourly());
        let (dir, store) = saved(&with_status(t.clone(), TaskStatus::Pending));
        let inbox = inbox_in(&dir);
        finish_worker(&store, &inbox, &t, &WorkerEnd::Attention, Utc::now());
        assert!(posted_workers(&dir).is_empty());
    }

    #[test]
    fn a_running_repeating_worker_needing_attention_posts_the_notice() {
        let t = task(worker(), hourly());
        let (dir, store) = saved(&t);
        let inbox = inbox_in(&dir);
        finish_worker(&store, &inbox, &t, &WorkerEnd::Attention, Utc::now());
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

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "x"]);
        dir
    }

    fn posted_texts(dir: &tempfile::TempDir) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(dir.path().join("inbox")) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| std::fs::read(entry.ok()?.path()).ok())
            .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .map(|value| value.to_string())
            .collect()
    }

    fn worker_task(repeat: Option<Repeat>) -> Task {
        Task {
            id: "x".to_string(),
            ..task(worker(), repeat)
        }
    }

    #[test]
    fn a_repeating_worker_with_an_open_branch_is_refused() {
        let repo = repo();
        git(repo.path(), &["branch", "worker/sched-x"]);
        let t = worker_task(hourly());
        let (dir, store) = saved(&t);
        let inbox = inbox_in(&dir);
        assert!(stop_if_open(
            |b| branch_open_scrubbed(repo.path(), b),
            &store,
            &inbox,
            &t,
            Utc::now()
        ));
        assert!(store.load_all().is_empty());
        assert_eq!(posted_workers(&dir), vec!["schedule:x".to_string()]);
        let text = "Scheduled worker x: the previous run's branch worker/sched-x is still open; repeat (every 1h) stopped. Land or close it, then propose the schedule again.";
        assert!(posted_texts(&dir).iter().any(|body| body.contains(text)));
    }

    #[test]
    fn a_repeating_worker_without_a_branch_is_not_refused() {
        let repo = repo();
        let t = worker_task(hourly());
        let (dir, store) = saved(&t);
        let inbox = inbox_in(&dir);
        assert!(!stop_if_open(
            |b| branch_open_scrubbed(repo.path(), b),
            &store,
            &inbox,
            &t,
            Utc::now()
        ));
        assert_eq!(store.load_all(), vec![t]);
        assert!(posted_workers(&dir).is_empty());
    }

    #[test]
    fn a_one_shot_worker_with_an_open_branch_is_not_refused() {
        let repo = repo();
        git(repo.path(), &["branch", "worker/sched-x"]);
        let t = worker_task(None);
        let (dir, store) = saved(&t);
        let inbox = inbox_in(&dir);
        assert!(!stop_if_open(
            |b| branch_open_scrubbed(repo.path(), b),
            &store,
            &inbox,
            &t,
            Utc::now()
        ));
        assert_eq!(store.load_all(), vec![t]);
    }
}
