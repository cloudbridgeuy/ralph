use crate::schedule::{
    format_span, watch_id, Task, TaskKind, TaskStatus, SCHEDULE_ID_PREFIX,
    WORKER_DEFAULT_TIMEOUT_SECS,
};
use crate::watch::WATCH_STATUS;
use crate::worker::{Report, ReportStatus};
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, Utc};
use std::time::Duration;

pub const WORKER_ID_PREFIX: &str = "sched-";
pub const WORKER_CAP: usize = 2;
pub const STARTUP_GRACE_SECS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerEnd {
    Done,
    Attention,
    TimedOut,
    Vanished,
}

#[derive(Debug, PartialEq, Eq)]
pub struct WorkerFire<'a> {
    pub task: &'a Task,
    pub late: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct WorkerTickPlan<'a> {
    pub ended: Vec<(&'a Task, WorkerEnd)>,
    pub fire: Vec<WorkerFire<'a>>,
}

pub fn worker_id(id: &str) -> String {
    format!("{WORKER_ID_PREFIX}{id}")
}

pub fn alive_with_starting(alive: &[String], starting: &[String]) -> Vec<String> {
    alive
        .iter()
        .cloned()
        .chain(starting.iter().map(|id| worker_id(id)))
        .collect()
}

pub fn is_still_running(tasks: &[Task], id: &str) -> bool {
    tasks
        .iter()
        .any(|task| task.id == id && task.status == TaskStatus::Running)
}

pub fn is_scheduled_worker_id(id: &str) -> bool {
    id.starts_with(WORKER_ID_PREFIX)
}

pub fn reply_refusal(id: &str, tasks: &[Task]) -> Option<String> {
    tasks
        .iter()
        .find(|task| is_worker_task(task) && worker_id(&task.id) == id)
        .map(|task| {
            format!(
                "Worker {id} belongs to scheduled task `{}`; it takes no replies. Cancel the task with /schedule cancel {}.",
                task.id, task.id
            )
        })
}

pub fn is_worker_task(task: &Task) -> bool {
    matches!(task.kind, TaskKind::Worker { .. })
}

pub fn source_matches_worker(source: &str, worker: &str) -> bool {
    source == worker
        || source
            .strip_prefix(SCHEDULE_ID_PREFIX)
            .is_some_and(|id| !id.is_empty() && worker == worker_id(id))
}

fn timeout_of(task: &Task) -> ChronoDuration {
    let secs = task.timeout_secs.unwrap_or(WORKER_DEFAULT_TIMEOUT_SECS);
    ChronoDuration::seconds(i64::try_from(secs).unwrap_or(i64::MAX))
}

pub fn worker_deadline(task: &Task, now: DateTime<Utc>) -> DateTime<Utc> {
    now.checked_add_signed(timeout_of(task)).unwrap_or(now)
}

fn started_at(task: &Task) -> DateTime<Utc> {
    task.deadline
        .and_then(|deadline| deadline.checked_sub_signed(timeout_of(task)))
        .unwrap_or(task.at)
}

pub fn worker_brief(task: &Task, now: DateTime<Utc>, offset: FixedOffset) -> Option<String> {
    let TaskKind::Worker { brief } = &task.kind else {
        return None;
    };
    Some(if task.late {
        format!(
            "Ran late: due {}, started {}.\n\n{brief}",
            task.at.with_timezone(&offset).format("%H:%M"),
            now.with_timezone(&offset).format("%H:%M"),
        )
    } else {
        brief.clone()
    })
}

fn report_end(task: &Task, reports: &[Report]) -> Option<WorkerEnd> {
    let worker = worker_id(&task.id);
    let started = started_at(task);
    reports
        .iter()
        .filter(|report| report.worker == worker)
        .filter(|report| DateTime::parse_from_rfc3339(&report.at).is_ok_and(|at| at >= started))
        .filter_map(|report| {
            ReportStatus::ALL
                .iter()
                .find(|status| status.as_str() == report.status)
        })
        .next_back()
        .map(|status| match status {
            ReportStatus::Done => WorkerEnd::Done,
            ReportStatus::Blocked | ReportStatus::Question => WorkerEnd::Attention,
        })
}

fn running_end(
    task: &Task,
    reports: &[Report],
    alive: &[String],
    now: DateTime<Utc>,
) -> Option<WorkerEnd> {
    let deadline = task
        .deadline
        .or_else(|| started_at(task).checked_add_signed(timeout_of(task)))?;
    report_end(task, reports).or_else(|| {
        if now >= deadline {
            Some(WorkerEnd::TimedOut)
        } else if !alive.contains(&worker_id(&task.id))
            && now >= started_at(task) + ChronoDuration::seconds(STARTUP_GRACE_SECS)
        {
            Some(WorkerEnd::Vanished)
        } else {
            None
        }
    })
}

fn by_time(tasks: &mut [&Task]) {
    tasks.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
}

pub fn plan_worker_tick<'a>(
    tasks: &'a [Task],
    reports: &[Report],
    alive: &[String],
    now: DateTime<Utc>,
    tick: Duration,
) -> WorkerTickPlan<'a> {
    let mut running: Vec<&Task> = tasks
        .iter()
        .filter(|task| is_worker_task(task) && task.status == TaskStatus::Running)
        .collect();
    by_time(&mut running);
    let (ended, still): (Vec<_>, Vec<_>) = running
        .into_iter()
        .map(|task| (task, running_end(task, reports, alive, now)))
        .partition(|(_, end)| end.is_some());
    let ended = ended
        .into_iter()
        .filter_map(|(task, end)| end.map(|end| (task, end)))
        .collect();
    let mut due: Vec<&Task> = tasks
        .iter()
        .filter(|task| is_worker_task(task) && task.status == TaskStatus::Pending)
        .filter(|task| task.at <= now)
        .collect();
    by_time(&mut due);
    let slots = WORKER_CAP.saturating_sub(still.len());
    let grace = ChronoDuration::from_std(tick).unwrap_or(ChronoDuration::MAX);
    WorkerTickPlan {
        ended,
        fire: due
            .into_iter()
            .take(slots)
            .map(|task| WorkerFire {
                task,
                late: task.late || now - task.at > grace,
            })
            .collect(),
    }
}

pub fn chat_report(task_id: &str, text: String, at: &str) -> Report {
    Report {
        worker: watch_id(task_id),
        at: at.to_string(),
        status: WATCH_STATUS.to_string(),
        text,
    }
}

pub fn start_failure_text(task_id: &str, error: &str) -> String {
    format!("Scheduled worker `{task_id}` did not start: {error}.")
}

pub fn timed_out_text(task: &Task, stop_error: Option<&str>) -> String {
    let stop = match stop_error {
        None => "Its harness was stopped.".to_string(),
        Some(error) => format!("Its harness could not be stopped: {error}."),
    };
    format!(
        "Scheduled worker `{}` timed out after {}. {stop} Worktree and branch {} are kept.",
        task.id,
        format_span(task.timeout_secs.unwrap_or(WORKER_DEFAULT_TIMEOUT_SECS)),
        crate::worker::branch_name(&worker_id(&task.id)),
    )
}

pub fn vanished_text(task: &Task) -> String {
    format!(
        "Scheduled worker `{}` stopped without a report. Worktree and branch {} are kept.",
        task.id,
        crate::worker::branch_name(&worker_id(&task.id)),
    )
}

pub fn new_scheduled_reports(known: &[Report], incoming: &[Report]) -> Vec<Report> {
    let mut fresh: Vec<Report> = Vec::new();
    for report in incoming {
        let relevant = report.worker.starts_with(WORKER_ID_PREFIX)
            && ReportStatus::ALL
                .iter()
                .any(|status| status.as_str() == report.status);
        let seen = known
            .iter()
            .chain(&fresh)
            .any(|old| old.worker == report.worker && old.at == report.at);
        if relevant && !seen {
            fresh.push(report.clone());
        }
    }
    fresh
}

#[cfg(test)]
mod tests;
