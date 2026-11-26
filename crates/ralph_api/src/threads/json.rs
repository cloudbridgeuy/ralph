use crate::render::graph_svg;
use crate::threads::cells::{lane_cells, Lane};
use chrono::{DateTime, SecondsFormat, Utc};
use ralph_core::chat_thread_log::Tombstone;
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

fn created_text(created: &str) -> String {
    DateTime::parse_from_rfc3339(created)
        .map(|at| {
            at.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        })
        .unwrap_or_else(|_| created.to_string())
}

fn thread_row(row: &ThreadRow, now: DateTime<Utc>) -> Value {
    json!({
        "kind": "thread",
        "name": row.name,
        "state": state_name(&row.state),
        "age": state_age(&row.state, now),
        "base": row.base,
        "created": created_text(&row.created),
        "active": row.active.as_deref().and_then(|at| ago(at, now)),
        "messages": row.messages,
        "workers": row.workers,
        "unread": row.unread,
        "svg": lane_svg(state_lane(&row.state)),
    })
}

fn lane_svg(lane: Lane) -> String {
    graph_svg(&lane_cells(lane))
}

fn state_lane(state: &ThreadState) -> Lane {
    match state {
        ThreadState::Open => Lane::Open,
        ThreadState::Parked(_) => Lane::Parked,
        ThreadState::Merged(_) => Lane::Merged,
    }
}

fn head_row(messages: usize) -> Value {
    json!({
        "kind": "head",
        "name": "main",
        "state": "head",
        "age": null,
        "base": null,
        "created": null,
        "active": null,
        "messages": messages,
        "workers": null,
        "unread": null,
        "svg": lane_svg(Lane::Head),
    })
}

fn dropped_row(tombstone: &Tombstone) -> Value {
    json!({
        "kind": "dropped",
        "name": tombstone.name,
        "state": "dropped",
        "age": null,
        "base": null,
        "created": null,
        "active": null,
        "messages": null,
        "workers": null,
        "unread": null,
        "svg": lane_svg(Lane::Dropped),
    })
}

fn dropped_in_name_order(tombstones: &[Tombstone]) -> Vec<&Tombstone> {
    let mut sorted: Vec<&Tombstone> = tombstones.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    sorted
}

pub(crate) fn threads_body(store: &str, snapshot: &ThreadsSnapshot, now: DateTime<Utc>) -> Value {
    let rows: Vec<Value> = std::iter::once(head_row(snapshot.head_messages))
        .chain(snapshot.rows.iter().map(|row| thread_row(row, now)))
        .chain(
            dropped_in_name_order(&snapshot.tombstones)
                .into_iter()
                .map(dropped_row),
        )
        .collect();
    json!({"store": store, "rows": rows})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::golden::assert_golden_in;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-07T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
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
            tombstones: vec![Tombstone {
                name: "old-idea".to_string(),
                base: 2,
                created: "2025-12-30T00:00:00Z".to_string(),
                dropped_at: "2026-01-06T00:00:00Z".to_string(),
            }],
        }
    }

    #[test]
    fn created_is_normalised_to_whole_seconds_with_a_z_suffix() {
        let real = row(
            "real",
            ThreadState::Open,
            1,
            "2026-01-02T03:04:05.123456789+00:00",
        );
        let offset = row("offset", ThreadState::Open, 1, "2026-01-02T05:04:05+02:00");
        let snapshot = ThreadsSnapshot {
            rows: vec![real, offset],
            head_messages: 1,
            tombstones: vec![],
        };
        let body = threads_body("chat", &snapshot, now());
        assert_eq!(rows(&body)[1]["created"], "2026-01-02T03:04:05Z");
        assert_eq!(rows(&body)[2]["created"], "2026-01-02T03:04:05Z");
    }

    #[test]
    fn created_that_does_not_parse_passes_through_unchanged() {
        let odd = row("odd", ThreadState::Open, 1, "yesterday-ish");
        let snapshot = ThreadsSnapshot {
            rows: vec![odd],
            head_messages: 1,
            tombstones: vec![],
        };
        let body = threads_body("chat", &snapshot, now());
        assert_eq!(rows(&body)[1]["created"], "yesterday-ish");
    }

    fn empty() -> ThreadsSnapshot {
        ThreadsSnapshot {
            rows: vec![],
            head_messages: 0,
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

    fn rows(body: &Value) -> &Vec<Value> {
        body["rows"].as_array().unwrap()
    }

    #[test]
    fn rows_follow_table_order_with_head_first_and_dropped_last() {
        let body = threads_body("chat", &fixture(), now());
        let order: Vec<(&str, &str, &str)> = rows(&body)
            .iter()
            .map(|row| {
                (
                    row["kind"].as_str().unwrap(),
                    row["name"].as_str().unwrap(),
                    row["state"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                ("head", "main", "head"),
                ("thread", "lint-sweep", "open"),
                ("thread", "docs-pass", "parked"),
                ("thread", "cache-fix", "merged"),
                ("dropped", "old-idea", "dropped"),
            ]
        );
        assert!(body.get("threads").is_none());
        assert!(body.get("graph").is_none());
        assert!(body.get("head").is_none());
    }

    #[test]
    fn dropped_rows_are_in_rows_with_null_counts() {
        let body = threads_body("chat", &fixture(), now());
        let dropped = &rows(&body)[4];
        for key in [
            "age", "base", "created", "active", "messages", "workers", "unread",
        ] {
            assert_eq!(dropped[key], Value::Null, "{key}");
        }
        let head = &rows(&body)[0];
        assert_eq!(head["messages"], 4);
        assert_eq!(head["base"], Value::Null);
    }

    #[test]
    fn every_row_carries_its_graph_svg() {
        let body = threads_body("chat", &fixture(), now());
        let lanes = [
            Lane::Head,
            Lane::Open,
            Lane::Parked,
            Lane::Merged,
            Lane::Dropped,
        ];
        assert_eq!(rows(&body).len(), lanes.len());
        for (row, lane) in rows(&body).iter().zip(lanes) {
            let expected = graph_svg(&lane_cells(lane));
            assert_eq!(row["svg"], expected.as_str(), "{}", row["name"]);
            assert!(expected.starts_with("<r-graph><svg"));
        }
    }

    #[test]
    fn no_row_carries_a_cells_key() {
        let body = threads_body("chat", &fixture(), now());
        assert_eq!(rows(&body).len(), 5);
        for row in rows(&body) {
            assert!(row.get("cells").is_none(), "{}", row["name"]);
            assert!(row.get("svg").is_some(), "{}", row["name"]);
        }
    }

    #[test]
    fn open_threads_have_null_age() {
        let body = threads_body("chat", &fixture(), now());
        assert_eq!(rows(&body)[1]["age"], Value::Null);
        assert_eq!(rows(&body)[2]["age"], "4d ago");
    }
}
