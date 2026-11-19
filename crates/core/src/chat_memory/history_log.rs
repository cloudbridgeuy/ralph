use super::{MemoryCall, Mode, SearchRequest, Tool};
use crate::chat_thread::{Scope, ThreadName};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

const HISTORY_VERSION: u32 = 1;
const ANALYTICS_DIR: &str = "analytics";
const HISTORY_FILE: &str = "history.jsonl";
pub const ARG_CHARS: usize = 200;
pub const ERROR_CHARS: usize = 300;
pub const VIEW_IDS_CAP: usize = 64;
const HASH_HEX_CHARS: usize = 16;

pub fn history_path(store: &Path) -> PathBuf {
    store.join(ANALYTICS_DIR).join(HISTORY_FILE)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn byte_count(text: &str) -> u64 {
    u64::try_from(text.len()).unwrap_or(u64::MAX)
}

pub fn cap_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut capped: String = text.chars().take(max).collect();
    capped.push('…');
    capped
}

#[derive(Debug, Clone, Copy)]
pub struct HistoryKey<'a> {
    pub store: &'a str,
    pub run: Option<&'a str>,
    pub at: DateTime<Utc>,
    pub turn: Option<&'a str>,
}

pub fn turn_id(run: &str, number: u64) -> String {
    format!("{run}#{number}")
}

pub fn parse_turn_id(text: &str) -> Option<(&str, u64)> {
    let (run, number) = text.trim().rsplit_once('#')?;
    if run.is_empty() {
        return None;
    }
    Some((run, number.parse().ok()?))
}

pub fn next_turn_number(current: Option<&str>, run: &str) -> u64 {
    match current.and_then(parse_turn_id) {
        Some((held, number)) if held == run => number.saturating_add(1),
        _ => 1,
    }
}

pub fn resolve_scope(scope: &Scope, fork: Option<&ThreadName>) -> Scope {
    match (scope, fork) {
        (Scope::Own, Some(name)) => Scope::Thread(name.clone()),
        (Scope::Own, None) => Scope::Main,
        (other, _) => other.clone(),
    }
}

pub fn view_line_ids(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| {
            let rest = line.strip_prefix('#')?;
            let id = rest.split_whitespace().next()?;
            rest.starts_with(char::is_alphanumeric)
                .then(|| id.to_string())
        })
        .collect()
}

pub fn count_turn_calls(text: &str, turn: &str) -> u64 {
    let counted = text
        .lines()
        .filter_map(|line| serde_json::from_str::<HistoryRow>(line).ok())
        .filter(|row| {
            row.turn.as_deref() == Some(turn)
                && matches!(
                    row.event,
                    HistoryEvent::Call {
                        source: CallSource::Mcp,
                        ..
                    }
                )
        })
        .count();
    u64::try_from(counted).unwrap_or(u64::MAX)
}

pub fn history_tool_named(name: &str) -> Option<HistoryTool> {
    [
        HistoryTool::Read,
        HistoryTool::Zoom,
        HistoryTool::Recall,
        HistoryTool::Search,
    ]
    .into_iter()
    .find(|tool| tool.name() == name)
}

#[derive(Debug, Clone, PartialEq)]
pub struct MalformedCall {
    pub tool: HistoryTool,
    pub raw: Value,
    pub message: String,
}

pub fn malformed_call(request: &Value, response: &Value) -> Option<MalformedCall> {
    let params = request.get("params")?;
    let tool = history_tool_named(params.get("name")?.as_str()?)?;
    let result = response.get("result")?;
    if result.get("isError")?.as_bool() != Some(true) {
        return None;
    }
    let message = result.get("content")?.get(0)?.get("text")?.as_str()?;
    Some(MalformedCall {
        tool,
        raw: params.get("arguments").cloned().unwrap_or(Value::Null),
        message: message.to_string(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallSource {
    Mcp,
    Slash,
    Cli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryTool {
    Read,
    Zoom,
    Recall,
    Search,
}

impl HistoryTool {
    pub fn name(self) -> &'static str {
        match self {
            HistoryTool::Read => "read",
            HistoryTool::Zoom => "zoom",
            HistoryTool::Recall => "recall",
            HistoryTool::Search => "search",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryOutcome {
    Ok,
    Empty,
    Error,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallMetrics {
    pub hits: Option<u64>,
    pub truncated: Option<bool>,
    pub node_size: Option<u64>,
    pub depth: Option<u64>,
    pub in_view: Option<bool>,
    pub oldest_hit: Option<u64>,
    pub top_cover_size: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub struct CallContext {
    pub source: CallSource,
    pub took: Duration,
    pub metrics: CallMetrics,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallFacts {
    source: CallSource,
    tool: HistoryTool,
    scope: String,
    args: Value,
    outcome: HistoryOutcome,
    error: Option<String>,
    result_bytes: u64,
    took: Duration,
    metrics: CallMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnFacts {
    pub scope: String,
    pub history_calls: u64,
    pub view_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewIds {
    Ids(Vec<String>),
    Hashed {
        hash: String,
        first: String,
        last: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum HistoryEvent {
    Call {
        source: CallSource,
        tool: HistoryTool,
        scope: String,
        args: Value,
        outcome: HistoryOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hits: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
        result_bytes: u64,
        duration_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_size: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        depth: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        in_view: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        oldest_hit: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        top_cover_size: Option<u64>,
    },
    Turn {
        scope: String,
        history_calls: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view_ids: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view_hash: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view_first: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view_last: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRow {
    v: u32,
    at: String,
    store: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn: Option<String>,
    #[serde(flatten)]
    event: HistoryEvent,
}

fn row(key: &HistoryKey, event: HistoryEvent) -> HistoryRow {
    HistoryRow {
        v: HISTORY_VERSION,
        at: key.at.to_rfc3339_opts(SecondsFormat::Secs, true),
        store: key.store.to_string(),
        run: key.run.map(str::to_string),
        turn: key.turn.map(str::to_string),
        event,
    }
}

pub fn scope_label(scope: &Scope) -> String {
    match scope {
        Scope::Own => "own".to_string(),
        Scope::Main => "main".to_string(),
        Scope::Thread(name) => name.as_str().to_string(),
    }
}

pub fn history_tool_of(tool: &Tool) -> HistoryTool {
    match tool {
        Tool::Zoom(_) => HistoryTool::Zoom,
        Tool::Recall(_) => HistoryTool::Recall,
        Tool::Read(_) => HistoryTool::Read,
        Tool::Search(_) => HistoryTool::Search,
    }
}

pub fn classify_call(result: &Result<String, String>) -> HistoryOutcome {
    match result {
        Err(_) => HistoryOutcome::Error,
        Ok(text) if text.trim().is_empty() || text.starts_with("no hits for ") => {
            HistoryOutcome::Empty
        }
        Ok(_) => HistoryOutcome::Ok,
    }
}

fn text_arg(raw: &Value, key: &str) -> Option<Value> {
    raw.get(key)
        .and_then(Value::as_str)
        .map(|s| Value::String(cap_text(s, ARG_CHARS)))
}

fn copied(raw: &Value, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .filter_map(|key| match raw.get(*key) {
            None | Some(Value::Null) => None,
            Some(Value::String(_)) => text_arg(raw, key).map(|v| (key.to_string(), v)),
            Some(other) => Some((key.to_string(), other.clone())),
        })
        .collect()
}

fn with_cursor(mut args: Map<String, Value>, raw: &Value) -> Value {
    args.insert(
        "has_cursor".to_string(),
        Value::Bool(raw.get("cursor").is_some_and(Value::is_string)),
    );
    Value::Object(args)
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Raw => "raw",
        Mode::Summary => "summary",
    }
}

pub fn call_args(tool: &Tool) -> Value {
    match tool {
        Tool::Zoom(node) => json!({"id": node.id()}),
        Tool::Recall(pattern) => json!({"pattern": cap_text(pattern, ARG_CHARS)}),
        Tool::Read(request) => json!({
            "from": request.from,
            "to": request.to,
            "mode": mode_name(request.mode),
            "budget": request.budget,
            "has_cursor": request.cursor.is_some(),
        }),
        Tool::Search(request) => search_args(request),
    }
}

fn search_args(request: &SearchRequest) -> Value {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String(cap_text(request.text(), ARG_CHARS)),
    );
    let texts = [
        ("role", request.role().map(|role| role.as_str())),
        ("tag", request.tag().map(|tag| tag.as_str())),
        ("since", request.since_text()),
        ("until", request.until_text()),
    ];
    for (key, text) in texts {
        if let Some(text) = text {
            args.insert(key.to_string(), Value::String(cap_text(text, ARG_CHARS)));
        }
    }
    args.insert("limit".to_string(), json!(request.limit()));
    args.insert("has_cursor".to_string(), Value::Bool(request.has_cursor()));
    Value::Object(args)
}

fn raw_args(tool: HistoryTool, raw: &Value) -> Value {
    match tool {
        HistoryTool::Zoom => Value::Object(copied(raw, &["id"])),
        HistoryTool::Recall => Value::Object(copied(raw, &["pattern"])),
        HistoryTool::Read => with_cursor(copied(raw, &["id", "mode", "budget"]), raw),
        HistoryTool::Search => with_cursor(
            copied(raw, &["query", "role", "tag", "since", "until", "limit"]),
            raw,
        ),
    }
}

fn raw_scope(raw: &Value) -> String {
    if let Some(thread) = raw.get("thread").and_then(Value::as_str) {
        return thread.trim().to_string();
    }
    raw.get("id")
        .and_then(Value::as_str)
        .and_then(|id| id.trim().trim_start_matches('#').split_once(':'))
        .map_or_else(|| "own".to_string(), |(scope, _)| scope.to_string())
}

pub fn history_call_facts(
    call: &MemoryCall,
    result: &Result<String, String>,
    ctx: CallContext,
) -> CallFacts {
    let (text, error) = match result {
        Ok(text) => (text, None),
        Err(message) => (message, Some(cap_text(message, ERROR_CHARS))),
    };
    CallFacts {
        source: ctx.source,
        tool: history_tool_of(&call.tool),
        scope: scope_label(&call.scope),
        args: call_args(&call.tool),
        outcome: classify_call(result),
        error,
        result_bytes: byte_count(text),
        took: ctx.took,
        metrics: ctx.metrics,
    }
}

fn call_event(facts: &CallFacts) -> HistoryEvent {
    let metrics = facts.metrics;
    HistoryEvent::Call {
        source: facts.source,
        tool: facts.tool,
        scope: facts.scope.clone(),
        args: facts.args.clone(),
        outcome: facts.outcome,
        error: facts.error.clone(),
        hits: metrics.hits,
        truncated: metrics.truncated,
        result_bytes: facts.result_bytes,
        duration_ms: millis(facts.took),
        node_size: metrics.node_size,
        depth: metrics.depth,
        in_view: metrics.in_view,
        oldest_hit: metrics.oldest_hit,
        top_cover_size: metrics.top_cover_size,
    }
}

pub fn history_call_row(key: &HistoryKey, facts: &CallFacts) -> HistoryRow {
    row(key, call_event(facts))
}

pub fn history_error_row(
    key: &HistoryKey,
    tool: HistoryTool,
    raw: &Value,
    message: &str,
    ctx: CallContext,
) -> HistoryRow {
    let facts = CallFacts {
        source: ctx.source,
        tool,
        scope: raw_scope(raw),
        args: raw_args(tool, raw),
        outcome: HistoryOutcome::Error,
        error: Some(cap_text(message, ERROR_CHARS)),
        result_bytes: 0,
        took: ctx.took,
        metrics: CallMetrics::default(),
    };
    history_call_row(key, &facts)
}

pub fn view_ids_field(ids: &[String]) -> ViewIds {
    let (Some(first), Some(last)) = (ids.first(), ids.last()) else {
        return ViewIds::Ids(Vec::new());
    };
    if ids.len() <= VIEW_IDS_CAP {
        return ViewIds::Ids(ids.to_vec());
    }
    let digest = Sha256::digest(ids.join("\n").as_bytes());
    let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    ViewIds::Hashed {
        hash: hash.chars().take(HASH_HEX_CHARS).collect(),
        first: first.clone(),
        last: last.clone(),
    }
}

pub fn history_turn_row(key: &HistoryKey, facts: &TurnFacts) -> HistoryRow {
    let (view_ids, view_hash, view_first, view_last) = match view_ids_field(&facts.view_ids) {
        ViewIds::Ids(ids) => (Some(ids), None, None, None),
        ViewIds::Hashed { hash, first, last } => (None, Some(hash), Some(first), Some(last)),
    };
    row(
        key,
        HistoryEvent::Turn {
            scope: facts.scope.clone(),
            history_calls: facts.history_calls,
            view_ids,
            view_hash,
            view_first,
            view_last,
        },
    )
}

pub fn encode_history(row: &HistoryRow) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(row)?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests;
