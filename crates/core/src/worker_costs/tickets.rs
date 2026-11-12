use super::window::instant;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

pub const UNKNOWN_TICKET: &str = "unknown";
const GRACE_SECS: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerWindow {
    pub worker_id: String,
    pub store: Option<String>,
    pub ticket: Option<String>,
    pub harness: String,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct EventLine {
    at: String,
    worker_id: String,
    store: Option<String>,
    event: String,
    ticket: Option<String>,
    harness: Option<String>,
}

impl WorkerWindow {
    fn in_store(&self, store: Option<&str>) -> bool {
        match (self.store.as_deref(), store) {
            (Some(own), Some(other)) => own == other,
            _ => true,
        }
    }

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
                store: line.store,
                ticket: line.ticket,
                harness: line.harness.unwrap_or_default(),
                start: at,
                end: None,
            }),
            "close" => {
                if let Some(open) = windows.iter_mut().rev().find(|window| {
                    window.worker_id == line.worker_id
                        && window.store == line.store
                        && window.end.is_none()
                }) {
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
    store: Option<&str>,
    worker_id: &str,
    at: DateTime<Utc>,
) -> Option<usize> {
    windows
        .iter()
        .enumerate()
        .filter(|(_, window)| {
            window.worker_id == worker_id && window.in_store(store) && window.covers(at)
        })
        .max_by_key(|(_, window)| window.start)
        .map(|(position, _)| position)
}

#[cfg(test)]
mod tests;
