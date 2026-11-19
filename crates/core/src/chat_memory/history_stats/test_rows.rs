use super::parse::{parse_history, Parsed};
use serde_json::{json, Value};

fn merged(mut base: Value, extra: Value) -> String {
    if let (Some(map), Some(more)) = (base.as_object_mut(), extra.as_object()) {
        map.extend(more.clone());
    }
    base.to_string()
}

pub fn call(extra: Value) -> String {
    merged(
        json!({
            "v": 1, "at": "2026-11-05T10:00:00Z", "store": "/s", "run": "r1", "turn": "r1#1",
            "event": "call", "source": "mcp", "tool": "search", "scope": "main",
            "args": {"query": "x"}, "outcome": "ok", "hits": 5,
            "result_bytes": 10, "duration_ms": 4
        }),
        extra,
    )
}

pub fn history(lines: &[String]) -> Parsed {
    parse_history(&lines.join("\n"))
}

pub fn at(second: u32) -> String {
    format!("2026-11-05T10:{:02}:{:02}Z", second / 60, second % 60)
}

pub fn turn(extra: Value) -> String {
    merged(
        json!({
            "v": 1, "at": "2026-11-05T10:05:00Z", "store": "/s", "run": "r1", "turn": "r1#1",
            "event": "turn", "scope": "main", "history_calls": 1, "view_ids": ["0-3", "4"]
        }),
        extra,
    )
}
