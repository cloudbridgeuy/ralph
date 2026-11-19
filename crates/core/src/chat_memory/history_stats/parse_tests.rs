use super::parse::*;
use serde_json::json;

fn call(extra: serde_json::Value) -> String {
    let mut row = json!({
        "v": 1, "at": "2026-11-05T10:00:00Z", "store": "/s", "run": "r1", "turn": "r1#1",
        "event": "call", "source": "mcp", "tool": "search", "scope": "main",
        "args": {"query": "x"}, "outcome": "ok", "hits": 3, "result_bytes": 10, "duration_ms": 4
    });
    if let (Some(map), Some(more)) = (row.as_object_mut(), extra.as_object()) {
        map.extend(more.clone());
    }
    row.to_string()
}

fn turn(extra: serde_json::Value) -> String {
    let mut row = json!({
        "v": 1, "at": "2026-11-05T10:00:05Z", "store": "/s", "run": "r1", "turn": "r1#1",
        "event": "turn", "scope": "main", "history_calls": 1, "view_ids": ["0-3", "4"]
    });
    if let (Some(map), Some(more)) = (row.as_object_mut(), extra.as_object()) {
        map.extend(more.clone());
    }
    row.to_string()
}

#[test]
fn parse_skips_malformed_lines_and_counts_them() {
    let good = call(json!({}));
    let half = &good[..good.len() / 2];
    let text = format!(
        "{good}\n\nnot json\n{}\n{}\n{half}",
        call(json!({"at": "yesterday"})),
        json!({"v": 1, "event": "call"})
    );
    let parsed = parse_history(&text);
    assert_eq!(parsed.calls.len(), 1);
    assert_eq!(parsed.skipped, 4);
}

#[test]
fn parse_ignores_unknown_fields_and_events() {
    let text = format!(
        "{}\n{}\n{}\n",
        call(json!({"future": [1, 2], "v": 2})),
        json!({"v": 1, "at": "2026-11-05T10:00:00Z", "event": "wave"}),
        call(json!({"tool": "teleport", "source": "pigeon", "outcome": "maybe"}))
    );
    let parsed = parse_history(&text);
    assert_eq!(parsed.calls.len(), 2);
    assert_eq!(parsed.skipped, 1);
    assert_eq!(parsed.calls[0].tool, Tool::Search);
    assert_eq!(parsed.calls[1].tool, Tool::Unknown);
    assert_eq!(parsed.calls[1].source, Source::Unknown);
    assert_eq!(parsed.calls[1].outcome, Outcome::Unknown);
    assert_eq!(parsed.calls[1].index, 2);
}

#[test]
fn parse_accepts_rows_missing_optional_fields() {
    let text = format!(
        "{}\n{}\n{}\n",
        json!({"at": "2026-11-05T10:00:00Z", "event": "call", "source": "cli", "tool": "zoom"}),
        json!({"at": "2026-11-05T10:00:01Z", "event": "turn"}),
        turn(json!({"view_ids": null, "view_hash": "ab", "view_first": "8-11"}))
    );
    let parsed = parse_history(&text);
    assert_eq!(parsed.skipped, 0);
    let first = &parsed.calls[0];
    assert_eq!(
        (first.run.clone(), first.turn.clone(), first.hits),
        (None, None, None)
    );
    assert_eq!(first.outcome, Outcome::Unknown);
    assert_eq!(parsed.turns[0].view, View::Ids(Vec::new()));
    assert_eq!(parsed.turns[1].view, View::First("8-11".to_string()));
    assert_eq!(parsed.turns[1].history_calls, Some(1));
}

#[test]
fn parse_reads_calls_and_turn_fields() {
    let text = format!(
        "{}\n{}\n",
        call(json!({"hit_ids": ["auth:5", "7"], "oldest_hit": 2, "depth": 3, "node_size": 8})),
        turn(json!({}))
    );
    let parsed = parse_history(&text);
    let first = &parsed.calls[0];
    assert_eq!(
        first.hit_ids,
        Some(vec!["auth:5".to_string(), "7".to_string()])
    );
    assert_eq!(first.oldest_hit, Some(2));
    assert_eq!(first.depth, Some(3));
    assert_eq!(first.node_size, Some(8));
    assert_eq!(first.at, 1_793_872_800);
    let row = &parsed.turns[0];
    assert_eq!(
        row.view,
        View::Ids(vec!["0-3".to_string(), "4".to_string()])
    );
    assert_eq!(row.turn.as_deref(), Some("r1#1"));
    assert_eq!(row.index, 1);
}

#[test]
fn parse_reads_timing_turn_rows_only() {
    let text = format!(
        "{}\n{}\n{}\nbroken\n{}\n",
        json!({"v": 1, "at": "2026-11-05T10:00:05Z", "event": "turn", "run": "r1", "turn_ms": 900, "ok": true}),
        json!({"v": 1, "at": "2026-11-05T10:01:00Z", "event": "nap", "total_ms": 5}),
        json!({"v": 1, "at": "2026-11-05T10:02:00Z", "event": "nap_call", "node": "0-3"}),
        json!({"v": 1, "event": "turn", "run": "r1"}),
    );
    let parsed = parse_timing(&text);
    assert_eq!(parsed.turns.len(), 1);
    assert_eq!(parsed.skipped, 2);
    assert_eq!(parsed.turns[0].run.as_deref(), Some("r1"));
    assert_eq!(parsed.turns[0].turn_ms, Some(900));
    assert_eq!(parsed.turns[0].index, 0);
}
