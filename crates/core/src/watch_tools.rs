use crate::chat_send_it::is_check_watch_id;
use crate::mcp::string_arg;
use crate::schedule::SCHEDULE_ID_PREFIX;
use crate::watch::watch_id;
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
        "description": "List this chat's watches, one line each: `<id> <state> <kind> <time left> <timeout> <started at> <command>`. State is queued (requested, not started yet), running, or saved (a persistent record that is not running). Kind is normal or persistent. It shows the same watches /watch list shows, plus queued and saved ones. Read-only; takes no arguments. Use it for watch status; never guess from the chat history.",
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

pub const SKIP_SCHEDULE: &str = "it is a scheduled task; use /schedule cancel.";
pub const SKIP_SEND_IT: &str = "it checks a /send-it step; /send-it off stops the run.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopTarget {
    Ids(Vec<String>),
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopStep {
    Stop(String),
    Dequeue(String),
    Skip { id: String, reason: &'static str },
    Unknown(String),
}

pub fn watch_stop_definition() -> Value {
    json!({
        "name": "watch_stop",
        "description": "Stop watches of this chat, like /watch stop: kill a running command, remove a persistent watch's saved record, or drop a queued request. A running watch posts its stopped report; a saved or queued one posts none. Give ids, or all: true to stop every watch except scheduled tasks and /send-it checks (the result names what it skipped). Only when the person asked for it in their latest message. Call watch_list first.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "ids": {"type": "array", "items": {"type": "string"}, "description": "Watch ids, `w1` or `watch:w1`."},
                "all": {"type": "boolean", "description": "Stop every watch. Use instead of ids. Default false."}
            }
        }
    })
}

pub fn parse_stop_args(args: &Value) -> Result<StopTarget, String> {
    let all = match args.get("all") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err("all must be true or false.".to_string()),
    };
    let ids = args.get("ids").filter(|value| !value.is_null());
    match (all, ids) {
        (true, None) => Ok(StopTarget::All),
        (true, Some(_)) => Err("Give ids or all, not both.".to_string()),
        (false, None) => Err("Give ids, or all: true.".to_string()),
        (false, Some(value)) => parse_ids(value).map(StopTarget::Ids),
    }
}

fn parse_ids(value: &Value) -> Result<Vec<String>, String> {
    let list = value
        .as_array()
        .ok_or_else(|| "ids must be a list of watch ids.".to_string())?;
    if list.is_empty() {
        return Err("ids must not be empty.".to_string());
    }
    let mut ids: Vec<String> = Vec::new();
    for item in list {
        let raw = string_arg(&json!({"id": item}), "id")?;
        if raw.chars().any(|c| c == '/' || c.is_whitespace()) {
            return Err(format!(
                "Watch id '{raw}' must not contain spaces or slashes."
            ));
        }
        let id = if raw.starts_with(SCHEDULE_ID_PREFIX) {
            raw
        } else {
            watch_id(&raw)
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

fn skip_reason(id: &str) -> Option<&'static str> {
    if id.starts_with(SCHEDULE_ID_PREFIX) {
        Some(SKIP_SCHEDULE)
    } else if is_check_watch_id(id) {
        Some(SKIP_SEND_IT)
    } else {
        None
    }
}

fn step_for(row: &WatchRow) -> StopStep {
    match row.state {
        WatchState::Queued => StopStep::Dequeue(row.id.clone()),
        WatchState::Running | WatchState::Saved => StopStep::Stop(row.id.clone()),
    }
}

pub fn plan_stop(target: &StopTarget, rows: &[WatchRow]) -> Vec<StopStep> {
    match target {
        StopTarget::All => rows
            .iter()
            .map(|row| match skip_reason(&row.id) {
                Some(reason) => StopStep::Skip {
                    id: row.id.clone(),
                    reason,
                },
                None => step_for(row),
            })
            .collect(),
        StopTarget::Ids(ids) => ids
            .iter()
            .map(|id| match rows.iter().find(|row| &row.id == id) {
                _ if id.starts_with(SCHEDULE_ID_PREFIX) => StopStep::Skip {
                    id: id.clone(),
                    reason: SKIP_SCHEDULE,
                },
                Some(row) => step_for(row),
                None => StopStep::Unknown(id.clone()),
            })
            .collect(),
    }
}

pub fn dequeued_message(id: &str) -> String {
    format!("Removed queued watch {id} before it started.")
}

pub fn skipped_message(id: &str, reason: &str) -> String {
    format!("Skipped {id}: {reason}")
}

pub fn stop_report(lines: &[String]) -> String {
    if lines.is_empty() {
        NO_WATCHES.to_string()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests;
