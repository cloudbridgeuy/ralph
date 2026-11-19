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

fn call(tool: Tool, result: Result<String, String>, ctx: CallContext) -> Value {
    let call = MemoryCall {
        scope: Scope::Own,
        tool,
    };
    value(&history_call_row(
        &key(),
        &history_call_facts(&call, &result, ctx),
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
    let args = call_args(&Tool::Read(request));
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
    let v = call(Tool::Recall("foo.*".into()), Ok("#1 hit".into()), ctx);
    assert_eq!(v["tool"], "recall");
    assert_eq!(v["args"], json!({"pattern": "foo.*"}));
    assert_eq!(v["hits"], 80);
    assert_eq!(v["truncated"], true);
    assert_eq!(v["oldest_hit"], 3);
    assert_eq!(v["top_cover_size"], 16);
}

#[test]
fn a_successful_search_row_logs_the_parsed_filters_and_cursor() {
    let request = parse_search_args(&json!({
        "query": "rate limit",
        "role": "user",
        "tag": "rule",
        "since": "2026-10-20",
        "until": "2026-10-21",
        "limit": 5,
        "cursor": "3.9.00000000000000ff",
    }))
    .unwrap();
    let tool = Tool::Search(request);
    let v = call(tool, Ok("x".into()), ctx());
    assert_eq!(
        v["args"],
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
fn a_successful_search_row_logs_the_default_limit() {
    let request = parse_search_args(&json!({"query": "x"})).unwrap();
    let tool = Tool::Search(request);
    let v = call(tool, Ok("x".into()), ctx());
    assert_eq!(
        v["args"],
        json!({"query": "x", "limit": 20, "has_cursor": false})
    );
}

#[test]
fn long_patterns_and_queries_are_capped() {
    let long = "é".repeat(ARG_CHARS + 50);
    let args = call_args(&Tool::Recall(long.clone()));
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
    let v = call(Tool::Recall("(".into()), Err("bad regex".into()), ctx());
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
    let facts = history_call_facts(&call, &Ok("x".into()), ctx());
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
        "hit_ids",
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
    let facts = history_call_facts(&call, &Ok("x".into()), ctx());
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
        &bad(
            HistoryTool::Search,
            json!({"query": "q", "thread": "auth", "cursor": "z"}),
            "bad cursor",
        ),
        ErrorScope::AsTyped,
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

fn bad(tool: HistoryTool, raw: Value, message: &str) -> MalformedCall {
    MalformedCall {
        tool,
        raw,
        message: message.to_string(),
    }
}

#[test]
fn an_error_row_resolves_an_untyped_scope_through_the_fork() {
    let auth = ThreadName::parse("auth").unwrap();
    let scope = |raw: Value, within: ErrorScope| {
        let row = history_error_row(&key(), &bad(HistoryTool::Zoom, raw, "x"), within, ctx());
        value(&row)["scope"].clone()
    };
    assert_eq!(scope(json!({}), ErrorScope::AsTyped), "own");
    assert_eq!(scope(json!({}), ErrorScope::Within(None)), "main");
    assert_eq!(scope(json!({}), ErrorScope::Within(Some(&auth))), "auth");
    assert_eq!(scope(Value::Null, ErrorScope::Within(Some(&auth))), "auth");
    let typed = json!({"thread": "other"});
    assert_eq!(scope(typed, ErrorScope::Within(Some(&auth))), "other");
    let prefixed = json!({"id": "main:4"});
    assert_eq!(scope(prefixed, ErrorScope::Within(Some(&auth))), "main");
}

#[test]
fn an_error_row_reads_the_scope_from_an_id_prefix() {
    let v = value(&history_error_row(
        &key(),
        &bad(HistoryTool::Zoom, json!({"id": "auth:4-7"}), "nope"),
        ErrorScope::AsTyped,
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
        &bad(HistoryTool::Zoom, json!({}), &long),
        ErrorScope::AsTyped,
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
fn a_turn_id_parses_back_into_its_parts() {
    assert_eq!(parse_turn_id("r1#3"), Some(("r1", 3)));
    assert_eq!(
        parse_turn_id(&turn_id("12-2026-11-19T10:00:00+00:00", 7)),
        Some(("12-2026-11-19T10:00:00+00:00", 7))
    );
    assert_eq!(parse_turn_id("r1#3\n"), Some(("r1", 3)));
    for bad in ["", "r1", "#3", "r1#", "r1#x"] {
        assert_eq!(parse_turn_id(bad), None, "{bad}");
    }
}

#[test]
fn the_next_turn_number_counts_on_within_a_run() {
    assert_eq!(next_turn_number(None, "r1"), 1);
    assert_eq!(next_turn_number(Some("r1#3"), "r1"), 4);
    assert_eq!(next_turn_number(Some("r0#9"), "r1"), 1);
    assert_eq!(next_turn_number(Some("junk"), "r1"), 1);
}

#[test]
fn own_resolves_to_the_fork_or_main() {
    let fork = ThreadName::parse("auth").unwrap();
    assert_eq!(
        scope_label(&resolve_scope(&Scope::Own, Some(&fork))),
        "auth"
    );
    assert_eq!(scope_label(&resolve_scope(&Scope::Own, None)), "main");
    assert_eq!(
        scope_label(&resolve_scope(&Scope::Main, Some(&fork))),
        "main"
    );
    let other = Scope::Thread(ThreadName::parse("db").unwrap());
    assert_eq!(scope_label(&resolve_scope(&other, Some(&fork))), "db");
}

#[test]
fn view_ids_come_from_the_numbered_lines() {
    let lines: Vec<String> = [
        "#0-15 summary of the start",
        "#16-23 more",
        "#24 user raw message",
        "auth: 2026-11-19",
        "#auth:3-4 a thread line",
        "# stray",
        "",
    ]
    .iter()
    .map(|line| line.to_string())
    .collect();
    assert_eq!(view_line_ids(&lines), ["0-15", "16-23", "24", "auth:3-4"]);
}

#[test]
fn only_call_rows_of_the_turn_are_counted() {
    let call = |turn: &str| {
        let call = MemoryCall {
            scope: Scope::Main,
            tool: Tool::Recall("a".into()),
        };
        let facts = history_call_facts(&call, &Ok("x".into()), ctx());
        let mut k = key();
        k.turn = Some(turn);
        encode_history(&history_call_row(&k, &facts)).unwrap()
    };
    let turn = encode_history(&history_turn_row(
        &key(),
        &TurnFacts {
            scope: "main".into(),
            history_calls: 0,
            view_ids: Vec::new(),
        },
    ))
    .unwrap();
    let text = format!(
        "{}{}{}{turn}not json\n",
        call("r1#3"),
        call("r1#4"),
        call("r1#3")
    );
    assert_eq!(count_turn_calls(&text, "r1#3"), 2);
    assert_eq!(count_turn_calls(&text, "r1#4"), 1);
    assert_eq!(count_turn_calls(&text, "r1#9"), 0);
    assert_eq!(count_turn_calls("", "r1#3"), 0);
}

#[test]
fn only_mcp_call_rows_are_counted() {
    let call = |source: CallSource| {
        let call = MemoryCall {
            scope: Scope::Main,
            tool: Tool::Recall("a".into()),
        };
        let mut ctx = ctx();
        ctx.source = source;
        let facts = history_call_facts(&call, &Ok("x".into()), ctx);
        encode_history(&history_call_row(&key(), &facts)).unwrap()
    };
    let text = format!(
        "{}{}{}{}",
        call(CallSource::Mcp),
        call(CallSource::Slash),
        call(CallSource::Cli),
        call(CallSource::Mcp)
    );
    assert_eq!(count_turn_calls(&text, "r1#3"), 2);
}

#[test]
fn history_tools_are_found_by_name() {
    assert_eq!(history_tool_named("zoom"), Some(HistoryTool::Zoom));
    assert_eq!(history_tool_named("search"), Some(HistoryTool::Search));
    assert_eq!(history_tool_named("changes"), None);
    assert_eq!(HistoryTool::Recall.name(), "recall");
}

#[test]
fn a_failed_history_tool_response_is_a_malformed_call() {
    let request =
        json!({"method": "tools/call", "params": {"name": "zoom", "arguments": {"x": 1}}});
    let failed = |is_error: bool| json!({"result": {"isError": is_error, "content": [{"type": "text", "text": "bad id"}]}});
    assert_eq!(
        malformed_call(&request, &failed(true)),
        Some(MalformedCall {
            tool: HistoryTool::Zoom,
            raw: json!({"x": 1}),
            message: "bad id".into(),
        })
    );
    assert_eq!(malformed_call(&request, &failed(false)), None);
    let other = json!({"method": "tools/call", "params": {"name": "reply", "arguments": {}}});
    assert_eq!(malformed_call(&other, &failed(true)), None);
    assert_eq!(
        malformed_call(&json!({"method": "ping"}), &failed(true)),
        None
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
        serde_json::from_str(&encode_history(&history_turn_row(&key(), &facts)).unwrap()).unwrap();
    assert_eq!(value(&back), v);
}

#[test]
fn a_search_row_lists_hit_ids_in_returned_order() {
    let mut ctx = ctx();
    ctx.metrics.hits = Some(3);
    ctx.metrics.hit_ids = vec!["7".into(), "auth:4".into(), "2".into()];
    let v = call(Tool::Recall("foo".into()), Ok("#7 hit".into()), ctx);
    assert_eq!(v["hit_ids"], json!(["7", "auth:4", "2"]));
}

#[test]
fn a_row_without_hit_ids_omits_the_field() {
    let v = call(Tool::Recall("foo".into()), Ok("no hits".into()), ctx());
    assert!(v.get("hit_ids").is_none());
}

#[test]
fn an_old_row_without_hit_ids_still_parses() {
    let line = r#"{"v":1,"at":"a","store":"/s","event":"call","source":"mcp","tool":"search","scope":"own","args":{},"outcome":"ok","hits":2,"result_bytes":4,"duration_ms":1}"#;
    let row: HistoryRow = serde_json::from_str(line).unwrap();
    assert!(value(&row).get("hit_ids").is_none());
}
