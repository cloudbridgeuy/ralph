use super::window::instant;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

pub const UNKNOWN_TICKET: &str = "unknown";
const GRACE_SECS: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerWindow {
    pub worker_id: String,
    pub ticket: Option<String>,
    pub harness: String,
    pub model: Option<String>,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct EventLine {
    at: String,
    worker_id: String,
    event: String,
    ticket: Option<String>,
    harness: Option<String>,
    model: Option<String>,
}

impl WorkerWindow {
    fn covers(&self, at: DateTime<Utc>) -> bool {
        self.start <= at
            && self
                .end
                .is_none_or(|end| at <= end + Duration::seconds(GRACE_SECS))
    }
}

pub fn parse_windows(workers_jsonl: &str) -> Vec<WorkerWindow> {
    let mut windows: Vec<WorkerWindow> = Vec::new();
    for line in workers_jsonl
        .lines()
        .filter_map(|line| serde_json::from_str::<EventLine>(line).ok())
    {
        let Some(at) = instant(&line.at) else {
            continue;
        };
        match line.event.as_str() {
            "spawn" => windows.push(WorkerWindow {
                worker_id: line.worker_id,
                ticket: line.ticket,
                harness: line.harness.unwrap_or_default(),
                model: line.model,
                start: at,
                end: None,
            }),
            "close" => {
                if let Some(open) = windows
                    .iter_mut()
                    .rev()
                    .find(|window| window.worker_id == line.worker_id && window.end.is_none())
                {
                    open.end = Some(at);
                }
            }
            _ => {}
        }
    }
    windows
}

pub fn window_position(
    windows: &[WorkerWindow],
    worker_id: &str,
    at: DateTime<Utc>,
) -> Option<usize> {
    windows
        .iter()
        .enumerate()
        .filter(|(_, window)| window.worker_id == worker_id && window.covers(at))
        .max_by_key(|(_, window)| window.start)
        .map(|(position, _)| position)
}

pub fn window_of<'a>(
    windows: &'a [WorkerWindow],
    worker_id: &str,
    at: DateTime<Utc>,
) -> Option<&'a WorkerWindow> {
    window_position(windows, worker_id, at).map(|position| &windows[position])
}

#[cfg(test)]
mod tests;
