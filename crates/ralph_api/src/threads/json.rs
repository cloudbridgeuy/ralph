use chrono::{DateTime, Utc};
use ralph_core::chat_thread_graph::{graph_cells, lanes_from, GraphRow, LaneEnd, RowKind, Tone};
use ralph_core::chat_thread_table::{ago, ThreadRow, ThreadState};
use ralph_server::chat::memory::ThreadsSnapshot;
use serde_json::{json, Value};

fn state_name(state: &ThreadState) -> &'static str {
    match state {
        ThreadState::Open => "open",
        ThreadState::Parked(_) => "parked",
        ThreadState::Merged(_) => "merged",
    }
}

fn state_age(state: &ThreadState, now: DateTime<Utc>) -> Option<String> {
    match state {
        ThreadState::Open => None,
        ThreadState::Parked(at) | ThreadState::Merged(at) => ago(at, now),
    }
}

fn thread_json(row: &ThreadRow, now: DateTime<Utc>) -> Value {
    json!({
        "name": row.name,
        "state": state_name(&row.state),
        "age": state_age(&row.state, now),
        "base": row.base,
        "created": row.created,
        "active": row.active.as_deref().and_then(|at| ago(at, now)),
        "messages": row.messages,
        "workers": row.workers,
        "unread": row.unread,
    })
}

fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Plain => "plain",
        Tone::Green => "green",
        Tone::Yellow => "yellow",
        Tone::Red => "red",
    }
}

fn graph_row_json(row: &GraphRow) -> Value {
    let (kind, name) = match &row.kind {
        RowKind::Trunk { .. } => ("head", None),
        RowKind::Tip {
            name,
            end: LaneEnd::Dropped,
        } => ("dropped", Some(name)),
        RowKind::Tip { name, .. } => ("thread", Some(name)),
    };
    json!({
        "kind": kind,
        "name": name,
        "label_start": row.label_start,
        "cells": row
            .cells
            .iter()
            .map(|(ch, tone)| json!({"ch": ch.to_string(), "tone": tone_name(*tone)}))
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn threads_body(store: &str, snapshot: &ThreadsSnapshot, now: DateTime<Utc>) -> Value {
    let lanes = lanes_from(&snapshot.live, &snapshot.tombstones);
    json!({
        "store": store,
        "head": {"messages": snapshot.head_messages},
        "threads": snapshot.rows.iter().map(|row| thread_json(row, now)).collect::<Vec<_>>(),
        "graph": graph_cells(Some(snapshot.head_messages), &lanes)
            .iter()
            .map(graph_row_json)
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::golden::assert_golden_in;
    use ralph_core::chat_thread::{Merged, Parked, ThreadMeta};
    use ralph_core::chat_thread_log::Tombstone;
    use ralph_core::chat_thread_table::{ThreadRow, ThreadState};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-07T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn meta(base: usize, created: &str) -> ThreadMeta {
        ThreadMeta {
            base,
            created: created.to_string(),
            merged: None,
            parked: None,
            auto: None,
            model: None,
            closed: false,
        }
    }

    fn row(name: &str, state: ThreadState, base: usize, created: &str) -> ThreadRow {
        ThreadRow {
            name: name.to_string(),
            state,
            base,
            created: created.to_string(),
            active: None,
            messages: 2,
            workers: 0,
            unread: 0,
        }
    }

    fn fixture() -> ThreadsSnapshot {
        let mut open = row("lint-sweep", ThreadState::Open, 2, "2026-01-02T00:00:00Z");
        open.active = Some("2026-01-06T23:00:00Z".to_string());
        open.workers = 1;
        open.unread = 3;
        let parked_at = "2026-01-03T00:00:00Z";
        let merged_at = "2026-01-05T00:00:00Z";
        let mut parked = meta(1, "2026-01-01T00:00:00Z");
        parked.parked = Some(Parked {
            at: parked_at.to_string(),
        });
        let mut merged = meta(3, "2026-01-04T00:00:00Z");
        merged.merged = Some(Merged {
            at: merged_at.to_string(),
            digest: "done".to_string(),
            landed: Some(4),
        });
        ThreadsSnapshot {
            rows: vec![
                open,
                row(
                    "docs-pass",
                    ThreadState::Parked(parked_at.to_string()),
                    1,
                    "2026-01-01T00:00:00Z",
                ),
                row(
                    "cache-fix",
                    ThreadState::Merged(merged_at.to_string()),
                    3,
                    "2026-01-04T00:00:00Z",
                ),
            ],
            head_messages: 4,
            live: vec![
                ("lint-sweep".to_string(), meta(2, "2026-01-02T00:00:00Z")),
                ("docs-pass".to_string(), parked),
                ("cache-fix".to_string(), merged),
            ],
            tombstones: vec![Tombstone {
                name: "old-idea".to_string(),
                base: 2,
                created: "2025-12-30T00:00:00Z".to_string(),
                dropped_at: "2026-01-06T00:00:00Z".to_string(),
            }],
        }
    }

    fn empty() -> ThreadsSnapshot {
        ThreadsSnapshot {
            rows: vec![],
            head_messages: 0,
            live: vec![],
            tombstones: vec![],
        }
    }

    fn pretty(value: &Value) -> String {
        serde_json::to_string_pretty(value).unwrap()
    }

    #[test]
    fn threads_body_golden() {
        let body = threads_body("chat", &fixture(), now());
        assert_golden_in("", "txt", "threads", &pretty(&body));
    }

    #[test]
    fn threads_body_empty_golden() {
        let body = threads_body("chat", &empty(), now());
        assert_golden_in("", "txt", "threads_empty", &pretty(&body));
    }

    #[test]
    fn dropped_threads_are_only_in_the_graph() {
        let body = threads_body("chat", &fixture(), now());
        let names: Vec<&str> = body["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|thread| thread["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["lint-sweep", "docs-pass", "cache-fix"]);
        let dropped = body["graph"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["kind"] == "dropped")
            .count();
        assert_eq!(dropped, 1);
    }

    #[test]
    fn open_threads_have_null_age() {
        let body = threads_body("chat", &fixture(), now());
        assert_eq!(body["threads"][0]["age"], Value::Null);
        assert_eq!(body["threads"][1]["age"], "4d ago");
    }
}
