use super::{MemoryCall, Mode, Tool};
use crate::chat_thread::Scope;
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

pub fn call_args(tool: &Tool, raw: &Value) -> Value {
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
        Tool::Search(_) => raw_args(HistoryTool::Search, raw),
    }
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
    raw: &Value,
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
        args: call_args(&call.tool, raw),
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
mod tests {
    use super::*;
    use crate::chat_memory::{parse_search_args, Mode, Node, RangeRequest, Tool};
    use crate::chat_thread::{Scope, ThreadName};
    use chrono::DateTime;
    use serde_json::{json, Value};
    use std::time::Duration;

    fn key() -> HistoryKey<'static> {
        HistoryKey {
            store: "/s",
            run: Some("r1"),
            at: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            turn: Some("r1#3"),
        }
    }

    fn ctx() -> CallContext {
        CallContext {
            source: CallSource::Mcp,
            took: Duration::from_millis(12),
            metrics: CallMetrics::default(),
        }
    }

    fn value(row: &HistoryRow) -> Value {
        let line = encode_history(row).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        serde_json::from_str(&line).unwrap()
    }

    fn call(tool: Tool, raw: Value, result: Result<String, String>, ctx: CallContext) -> Value {
        let call = MemoryCall {
            scope: Scope::Own,
            tool,
        };
        value(&history_call_row(
            &key(),
            &history_call_facts(&call, &raw, &result, ctx),
        ))
    }

    #[test]
    fn the_log_sits_in_the_analytics_dir() {
        assert_eq!(
            history_path(std::path::Path::new("/s")),
            std::path::Path::new("/s/analytics/history.jsonl")
        );
    }

    #[test]
    fn a_zoom_row_carries_the_node_and_metrics() {
        let mut ctx = ctx();
        ctx.metrics.node_size = Some(4);
        ctx.metrics.depth = Some(2);
        ctx.metrics.in_view = Some(true);
        let v = call(
            Tool::Zoom(Node::parse("4-7").unwrap()),
            json!({"id": "4-7"}),
            Ok("abc".into()),
            ctx,
        );
        assert_eq!(
            v,
            json!({
                "v": 1,
                "at": "2023-11-14T22:13:20Z",
                "store": "/s",
                "run": "r1",
                "turn": "r1#3",
                "event": "call",
                "source": "mcp",
                "tool": "zoom",
                "scope": "own",
                "args": {"id": "4-7"},
                "outcome": "ok",
                "result_bytes": 3,
                "duration_ms": 12,
                "node_size": 4,
                "depth": 2,
                "in_view": true,
            })
        );
    }

    #[test]
    fn a_read_row_records_mode_budget_and_cursor() {
        let request = RangeRequest {
            from: 2,
            to: 6,
            mode: Mode::Summary,
            budget: 512,
            cursor: None,
        };
        let args = call_args(&Tool::Read(request), &json!({"id": "2-5"}));
        assert_eq!(
            args,
            json!({"from": 2, "to": 6, "mode": "summary", "budget": 512, "has_cursor": false})
        );
        let v = call(
            Tool::Read(RangeRequest {
                from: 2,
                to: 6,
                mode: Mode::Raw,
                budget: 4096,
                cursor: None,
            }),
            json!({"id": "2-5"}),
            Ok("x".into()),
            ctx(),
        );
        assert_eq!(v["tool"], "read");
        assert_eq!(v["args"]["mode"], "raw");
    }

    #[test]
    fn a_recall_row_records_the_pattern_and_truncation() {
        let mut ctx = ctx();
        ctx.metrics.hits = Some(80);
        ctx.metrics.truncated = Some(true);
        ctx.metrics.oldest_hit = Some(3);
        ctx.metrics.top_cover_size = Some(16);
        let v = call(
            Tool::Recall("foo.*".into()),
            json!({"pattern": "foo.*"}),
            Ok("#1 hit".into()),
            ctx,
        );
        assert_eq!(v["tool"], "recall");
        assert_eq!(v["args"], json!({"pattern": "foo.*"}));
        assert_eq!(v["hits"], 80);
        assert_eq!(v["truncated"], true);
        assert_eq!(v["oldest_hit"], 3);
        assert_eq!(v["top_cover_size"], 16);
    }

    #[test]
    fn a_search_row_records_the_filters_from_the_raw_args() {
        let raw = json!({
            "query": "rate limit",
            "role": "user",
            "tag": "rule",
            "since": "2026-10-20",
            "until": "2026-10-21",
            "limit": 5,
            "cursor": "abc",
            "thread": "auth",
        });
        let request = parse_search_args(&json!({"query": "rate limit"})).unwrap();
        let args = call_args(&Tool::Search(request), &raw);
        assert_eq!(
            args,
            json!({
                "query": "rate limit",
                "role": "user",
                "tag": "rule",
                "since": "2026-10-20",
                "until": "2026-10-21",
                "limit": 5,
                "has_cursor": true,
            })
        );
    }

    #[test]
    fn a_search_without_filters_has_only_the_query() {
        let request = parse_search_args(&json!({"query": "x"})).unwrap();
        let args = call_args(&Tool::Search(request), &json!({"query": "x"}));
        assert_eq!(args, json!({"query": "x", "has_cursor": false}));
    }

    #[test]
    fn long_patterns_and_queries_are_capped() {
        let long = "é".repeat(ARG_CHARS + 50);
        let args = call_args(&Tool::Recall(long.clone()), &json!({}));
        let capped = args["pattern"].as_str().unwrap();
        assert_eq!(capped.chars().count(), ARG_CHARS + 1);
        assert!(capped.ends_with('…'));
        assert_eq!(cap_text("short", 10), "short");
    }

    #[test]
    fn the_classifier_separates_ok_empty_and_error() {
        assert_eq!(classify_call(&Ok("#1 hit".into())), HistoryOutcome::Ok);
        assert_eq!(
            classify_call(&Ok("no hits for foo".into())),
            HistoryOutcome::Empty
        );
        assert_eq!(classify_call(&Ok("  \n".into())), HistoryOutcome::Empty);
        assert_eq!(classify_call(&Err("boom".into())), HistoryOutcome::Error);
    }

    #[test]
    fn an_error_result_sets_the_error_and_skips_hits() {
        let v = call(
            Tool::Recall("(".into()),
            json!({"pattern": "("}),
            Err("bad regex".into()),
            ctx(),
        );
        assert_eq!(v["outcome"], "error");
        assert_eq!(v["error"], "bad regex");
        assert_eq!(v["result_bytes"], 9);
        assert!(v.get("hits").is_none());
    }

    #[test]
    fn optional_fields_are_skipped_when_none() {
        let mut anon = key();
        anon.run = None;
        anon.turn = None;
        let call = MemoryCall {
            scope: Scope::Own,
            tool: Tool::Recall("a".into()),
        };
        let facts = history_call_facts(&call, &json!({}), &Ok("x".into()), ctx());
        let v = value(&history_call_row(&anon, &facts));
        for field in [
            "run",
            "turn",
            "error",
            "hits",
            "truncated",
            "node_size",
            "depth",
            "in_view",
            "oldest_hit",
            "top_cover_size",
        ] {
            assert!(v.get(field).is_none(), "{field}");
        }
    }

    #[test]
    fn the_scope_names_main_threads_and_own() {
        assert_eq!(scope_label(&Scope::Own), "own");
        assert_eq!(scope_label(&Scope::Main), "main");
        let thread = Scope::Thread(ThreadName::parse("auth").unwrap());
        assert_eq!(scope_label(&thread), "auth");
    }

    #[test]
    fn a_call_row_round_trips() {
        let call = MemoryCall {
            scope: Scope::Main,
            tool: Tool::Recall("a".into()),
        };
        let facts = history_call_facts(&call, &json!({}), &Ok("x".into()), ctx());
        let row = history_call_row(&key(), &facts);
        let back: HistoryRow = serde_json::from_str(&encode_history(&row).unwrap()).unwrap();
        assert_eq!(back, row);
        assert_eq!(value(&row)["scope"], "main");
    }

    #[test]
    fn readers_ignore_unknown_fields() {
        let line = r#"{"v":1,"at":"a","store":"/s","event":"turn","scope":"main","history_calls":0,"view_ids":[],"future":1}"#;
        let row: HistoryRow = serde_json::from_str(line).unwrap();
        assert_eq!(value(&row)["event"], "turn");
    }

    #[test]
    fn an_error_row_is_an_error_with_no_hits() {
        let v = value(&history_error_row(
            &key(),
            HistoryTool::Search,
            &json!({"query": "q", "thread": "auth", "cursor": "z"}),
            "bad cursor",
            CallContext {
                source: CallSource::Slash,
                ..ctx()
            },
        ));
        assert_eq!(v["event"], "call");
        assert_eq!(v["source"], "slash");
        assert_eq!(v["tool"], "search");
        assert_eq!(v["scope"], "auth");
        assert_eq!(v["outcome"], "error");
        assert_eq!(v["error"], "bad cursor");
        assert_eq!(v["result_bytes"], 0);
        assert_eq!(v["args"], json!({"query": "q", "has_cursor": true}));
        assert!(v.get("hits").is_none());
    }

    #[test]
    fn an_error_row_reads_the_scope_from_an_id_prefix() {
        let v = value(&history_error_row(
            &key(),
            HistoryTool::Zoom,
            &json!({"id": "auth:4-7"}),
            "nope",
            ctx(),
        ));
        assert_eq!(v["scope"], "auth");
        assert_eq!(v["args"], json!({"id": "auth:4-7"}));
    }

    #[test]
    fn long_errors_are_capped() {
        let long = "x".repeat(ERROR_CHARS + 10);
        let v = value(&history_error_row(
            &key(),
            HistoryTool::Zoom,
            &json!({}),
            &long,
            ctx(),
        ));
        assert_eq!(
            v["error"].as_str().unwrap().chars().count(),
            ERROR_CHARS + 1
        );
    }

    #[test]
    fn a_turn_id_joins_the_run_and_the_number() {
        assert_eq!(turn_id("r1", 3), "r1#3");
    }

    #[test]
    fn a_turn_row_lists_the_view_ids() {
        let facts = TurnFacts {
            scope: "main".into(),
            history_calls: 2,
            view_ids: vec!["0-15".into(), "16-23".into(), "24".into()],
        };
        assert_eq!(
            value(&history_turn_row(&key(), &facts)),
            json!({
                "v": 1,
                "at": "2023-11-14T22:13:20Z",
                "store": "/s",
                "run": "r1",
                "turn": "r1#3",
                "event": "turn",
                "scope": "main",
                "history_calls": 2,
                "view_ids": ["0-15", "16-23", "24"],
            })
        );
    }

    #[test]
    fn the_view_ids_stay_whole_up_to_the_cap() {
        let ids: Vec<String> = (0..VIEW_IDS_CAP).map(|i| i.to_string()).collect();
        assert_eq!(view_ids_field(&ids), ViewIds::Ids(ids.clone()));
    }

    #[test]
    fn more_ids_than_the_cap_fall_back_to_a_hash_and_bounds() {
        let ids: Vec<String> = (0..=VIEW_IDS_CAP).map(|i| i.to_string()).collect();
        let ViewIds::Hashed { hash, first, last } = view_ids_field(&ids) else {
            panic!("expected a hash");
        };
        assert_eq!(first, "0");
        assert_eq!(last, VIEW_IDS_CAP.to_string());
        assert_eq!(hash.len(), 16);
        let mut other = ids.clone();
        other[10] = "x".into();
        let ViewIds::Hashed { hash: again, .. } = view_ids_field(&other) else {
            panic!("expected a hash");
        };
        assert_ne!(hash, again);
    }

    #[test]
    fn a_hashed_turn_row_has_no_id_list() {
        let facts = TurnFacts {
            scope: "main".into(),
            history_calls: 0,
            view_ids: (0..=VIEW_IDS_CAP).map(|i| i.to_string()).collect(),
        };
        let v = value(&history_turn_row(&key(), &facts));
        assert!(v.get("view_ids").is_none());
        assert_eq!(v["view_first"], "0");
        assert_eq!(v["view_last"], VIEW_IDS_CAP.to_string());
        assert_eq!(v["view_hash"].as_str().unwrap().len(), 16);
        let back: HistoryRow =
            serde_json::from_str(&encode_history(&history_turn_row(&key(), &facts)).unwrap())
                .unwrap();
        assert_eq!(value(&back), v);
    }
}
