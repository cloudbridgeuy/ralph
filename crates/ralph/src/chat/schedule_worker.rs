use crate::chat::monitor::{Monitor, POLL};
use crate::warn::warn_if_err;
use crate::watches::locked;
use crate::workers::{running_workers, SpawnArgs, Workers};
use ralph_core::schedule::{Task, TaskStatus};
use ralph_core::schedule_worker::{
    alive_with_starting, chat_report, is_still_running, is_worker_task, plan_worker_tick,
    start_failure_text, timed_out_text, vanished_text, worker_brief, worker_deadline, worker_id,
    WorkerEnd, WorkerFire,
};
use ralph_core::worker::HarnessId;
use std::path::Path;
use std::sync::{Arc, Mutex};

static STARTING: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Starting(String);

impl Starting {
    fn enter(id: &str) -> Self {
        locked(&STARTING).push(id.to_string());
        Self(id.to_string())
    }
}

impl Drop for Starting {
    fn drop(&mut self) {
        locked(&STARTING).retain(|id| id != &self.0);
    }
}

pub(super) fn alive_worker_ids(store_dir: &Path) -> Vec<String> {
    running_workers(store_dir)
        .into_iter()
        .map(|worker| worker.id)
        .collect()
}

pub(super) fn tick_workers(monitor: &Monitor) {
    let tasks = monitor.schedule.load_all();
    if !tasks.iter().any(is_worker_task) || monitor.watches.is_closing() {
        return;
    }
    let any_running = tasks
        .iter()
        .any(|task| is_worker_task(task) && task.status == TaskStatus::Running);
    let alive = if any_running {
        alive_with_starting(
            &alive_worker_ids(&monitor.store_dir),
            &locked(&STARTING).clone(),
        )
    } else {
        Vec::new()
    };
    let reports = locked(&monitor.scheduled_reports).clone();
    let now = chrono::Utc::now();
    let plan = plan_worker_tick(&tasks, &reports, &alive, now, POLL);
    for (task, end) in &plan.ended {
        end_task(monitor, task, end, now);
    }
    for fire in plan.fire {
        start_task(monitor, fire, now);
    }
}

fn post_report(monitor: &Monitor, task: &Task, text: String, now: chrono::DateTime<chrono::Utc>) {
    let report = chat_report(&task.id, text, &now.to_rfc3339());
    warn_if_err(
        monitor.inbox.post(&report, 0),
        format!("posting schedule {}", task.id),
    );
}

fn end_task(monitor: &Monitor, task: &Task, end: &WorkerEnd, now: chrono::DateTime<chrono::Utc>) {
    let worker = worker_id(&task.id);
    match end {
        WorkerEnd::Done => settle_done(&monitor.workers, &worker),
        WorkerEnd::Attention => {}
        WorkerEnd::TimedOut => {
            let stop_error = monitor
                .workers
                .stop_harness(&worker)
                .err()
                .map(|error| error.to_string());
            if let Some(error) = &stop_error {
                crate::warn::warn(format!("stopping timed out worker {worker}: {error}"));
            }
            post_report(
                monitor,
                task,
                timed_out_text(task, stop_error.as_deref()),
                now,
            );
        }
        WorkerEnd::Vanished => post_report(monitor, task, vanished_text(task), now),
    }
    monitor.schedule.remove(&task.id);
}

fn settle_done(workers: &Workers, worker: &str) {
    if workers.close(worker, false, false).is_err() {
        warn_if_err(
            workers.stop_harness(worker),
            format!("stopping finished worker {worker}"),
        );
    }
}

fn start_task(monitor: &Monitor, fire: WorkerFire, now: chrono::DateTime<chrono::Utc>) {
    let task = Task {
        status: TaskStatus::Running,
        late: fire.late,
        deadline: Some(worker_deadline(fire.task, now)),
        ..fire.task.clone()
    };
    let offset = *chrono::Local::now().offset();
    let Some(brief) = worker_brief(&task, now, offset) else {
        return;
    };
    if let Err(error) = monitor.schedule.save(&task) {
        crate::warn::warn(format!(
            "marking scheduled task {} running: {error}",
            task.id
        ));
        return;
    }
    let workers = monitor.workers.clone();
    let store = monitor.schedule.clone();
    let inbox = Arc::clone(&monitor.inbox);
    let starting = Starting::enter(&task.id);
    std::thread::spawn(move || {
        let _starting = starting;
        let worker = worker_id(&task.id);
        if !is_still_running(&store.load_all(), &task.id) {
            return;
        }
        let args = SpawnArgs {
            id: &worker,
            task: &brief,
            harness: HarnessId::Claude,
            model: None,
            ticket: None,
            scheduled: true,
        };
        let spawned = workers.spawn(&args);
        let wanted = is_still_running(&store.load_all(), &task.id);
        match (spawned, wanted) {
            (Ok(_), false) => warn_if_err(
                workers.stop_harness(&worker),
                format!("stopping orphaned worker {worker}"),
            ),
            (Err(_), false) | (Ok(_), true) => {}
            (Err(error), true) => {
                let text = start_failure_text(&task.id, &error.to_string());
                let report = chat_report(&task.id, text, &now.to_rfc3339());
                warn_if_err(
                    inbox.post(&report, 0),
                    format!("posting schedule {}", task.id),
                );
                store.remove(&task.id);
            }
        }
    });
}
