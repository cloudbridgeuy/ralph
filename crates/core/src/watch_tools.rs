use crate::watch_command::{format_left, time_left};
use crate::watch_request::WatchRequest;
use crate::watch_store::SavedWatch;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::time::Duration;

const NO_WATCHES: &str = "No active watches.";
const COMMAND_CHARS: usize = 80;
const NONE: &str = "-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchState {
    Queued,
    Running,
    Saved,
}

impl WatchState {
    fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Saved => "saved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRow {
    pub id: String,
    pub state: WatchState,
    pub persistent: bool,
    pub timeout_secs: Option<u64>,
    pub left: Option<Duration>,
    pub started_at: Option<DateTime<Utc>>,
    pub command: String,
}

pub fn watch_list_definition() -> Value {
    json!({
        "name": "watch_list",
        "description": "List this chat's watches, one line each: `<id> <state> <kind> <time left> <timeout> <started at> <command>`. State is queued (requested, not started yet), running, or saved (a persistent record that is not running). Kind is normal or persistent. It shows the same watches /watch list shows, plus queued ones. Read-only; takes no arguments. Use it for watch status; never guess from the chat history.",
        "inputSchema": {"type": "object", "properties": {}}
    })
}

fn row(saved: &SavedWatch, state: WatchState, now: DateTime<Utc>) -> WatchRow {
    WatchRow {
        id: saved.id.clone(),
        state,
        persistent: saved.kind == crate::watch::WatchKind::Persistent,
        timeout_secs: saved.timeout_secs,
        left: time_left(saved, now),
        started_at: Some(saved.started_at),
        command: saved.command.clone(),
    }
}

fn queued_row(request: &WatchRequest) -> WatchRow {
    WatchRow {
        id: request.id.clone(),
        state: WatchState::Queued,
        persistent: request.persistent,
        timeout_secs: request.timeout_secs,
        left: None,
        started_at: None,
        command: request.command.clone(),
    }
}

pub fn merge_rows(
    queued: &[WatchRequest],
    running: &[SavedWatch],
    saved: &[SavedWatch],
    now: DateTime<Utc>,
) -> Vec<WatchRow> {
    let mut rows: Vec<WatchRow> = running
        .iter()
        .map(|watch| row(watch, WatchState::Running, now))
        .collect();
    for watch in saved {
        if !rows.iter().any(|r| r.id == watch.id) {
            rows.push(row(watch, WatchState::Saved, now));
        }
    }
    for request in queued {
        if !rows.iter().any(|r| r.id == request.id) {
            rows.push(queued_row(request));
        }
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

pub fn short_command(command: &str) -> String {
    let flat = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= COMMAND_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(COMMAND_CHARS - 1).collect();
    format!("{head}…")
}

fn timeout_label(row: &WatchRow) -> String {
    match (row.timeout_secs, row.state) {
        (Some(secs), _) => format!("{secs}s"),
        (None, WatchState::Queued) => "default".to_string(),
        (None, _) => "none".to_string(),
    }
}

pub fn format_rows(rows: &[WatchRow]) -> String {
    if rows.is_empty() {
        return NO_WATCHES.to_string();
    }
    rows.iter()
        .map(|row| {
            format!(
                "{} {} {} {} {} {} {}",
                row.id,
                row.state.label(),
                if row.persistent {
                    "persistent"
                } else {
                    "normal"
                },
                row.left.map_or_else(|| NONE.to_string(), format_left),
                timeout_label(row),
                row.started_at
                    .map_or_else(|| NONE.to_string(), |at| at.to_rfc3339()),
                short_command(&row.command),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;
