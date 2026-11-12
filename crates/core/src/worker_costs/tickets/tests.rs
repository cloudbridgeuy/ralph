use super::*;
use serde_json::{json, Value};

fn at(text: &str) -> DateTime<Utc> {
    instant(text).unwrap()
}

fn spawn(id: &str, when: &str, ticket: Option<&str>) -> Value {
    json!({
        "v": 1, "at": when, "store": "/s", "worker_id": id, "event": "spawn",
        "ticket": ticket, "harness": "claude", "model": "claude-opus-5-5",
    })
}

fn close(id: &str, when: &str) -> Value {
    json!({
        "v": 1, "at": when, "store": "/s", "worker_id": id, "event": "close",
        "outcome": "merged",
    })
}

fn jsonl(events: &[Value]) -> String {
    events.iter().map(|event| format!("{event}\n")).collect()
}

fn ticket_at(windows: &[WorkerWindow], id: &str, when: &str) -> Option<Option<String>> {
    window_position(windows, None, id, at(when)).map(|position| windows[position].ticket.clone())
}

#[test]
fn a_spawn_and_close_make_one_closed_window() {
    let windows = parse_windows(&jsonl(&[
        spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1")),
        close("w1", "2026-11-01T11:00:00Z"),
    ]));
    assert_eq!(
        windows,
        [WorkerWindow {
            worker_id: "w1".into(),
            store: Some("/s".into()),
            ticket: Some("GUZ-1".into()),
            harness: "claude".into(),
            start: at("2026-11-01T10:00:00Z"),
            end: Some(at("2026-11-01T11:00:00Z")),
        }]
    );
}

#[test]
fn a_spawn_without_a_close_stays_open() {
    let windows = parse_windows(&jsonl(&[spawn(
        "w1",
        "2026-11-01T10:00:00Z",
        Some("GUZ-1"),
    )]));
    assert_eq!(windows[0].end, None);
    assert_eq!(
        ticket_at(&windows, "w1", "2030-01-01T00:00:00Z"),
        Some(Some("GUZ-1".into()))
    );
}

#[test]
fn a_row_before_the_spawn_matches_no_window() {
    let windows = parse_windows(&jsonl(&[spawn(
        "w1",
        "2026-11-01T10:00:00Z",
        Some("GUZ-1"),
    )]));
    assert_eq!(ticket_at(&windows, "w1", "2026-11-01T09:59:59Z"), None);
}

#[test]
fn a_row_one_second_after_the_close_still_matches() {
    let windows = parse_windows(&jsonl(&[
        spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1")),
        close("w1", "2026-11-01T11:00:00Z"),
    ]));
    assert_eq!(
        ticket_at(&windows, "w1", "2026-11-01T11:00:01Z"),
        Some(Some("GUZ-1".into()))
    );
    assert_eq!(ticket_at(&windows, "w1", "2026-11-01T11:00:02Z"), None);
}

#[test]
fn a_reused_worker_id_joins_each_row_to_its_own_window() {
    let windows = parse_windows(&jsonl(&[
        spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1")),
        close("w1", "2026-11-01T11:00:00Z"),
        spawn("w1", "2026-11-02T10:00:00Z", Some("GUZ-2")),
        close("w1", "2026-11-02T11:00:00Z"),
    ]));
    assert_eq!(windows.len(), 2);
    assert_eq!(
        ticket_at(&windows, "w1", "2026-11-01T10:30:00Z"),
        Some(Some("GUZ-1".into()))
    );
    assert_eq!(
        ticket_at(&windows, "w1", "2026-11-02T10:30:00Z"),
        Some(Some("GUZ-2".into()))
    );
    assert_eq!(ticket_at(&windows, "w1", "2026-11-01T12:00:00Z"), None);
}

#[test]
fn a_close_ends_the_latest_open_window_of_its_worker() {
    let windows = parse_windows(&jsonl(&[
        spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1")),
        spawn("w2", "2026-11-01T10:00:00Z", Some("GUZ-9")),
        close("w1", "2026-11-01T11:00:00Z"),
    ]));
    assert!(windows[0].end.is_some());
    assert_eq!(windows[1].end, None);
}

#[test]
fn rows_of_another_worker_do_not_match() {
    let windows = parse_windows(&jsonl(&[spawn(
        "w1",
        "2026-11-01T10:00:00Z",
        Some("GUZ-1"),
    )]));
    assert_eq!(ticket_at(&windows, "w2", "2026-11-01T10:30:00Z"), None);
}

#[test]
fn a_spawn_without_a_ticket_keeps_the_window_with_no_ticket() {
    let windows = parse_windows(&jsonl(&[spawn("w1", "2026-11-01T10:00:00Z", None)]));
    assert_eq!(
        ticket_at(&windows, "w1", "2026-11-01T10:30:00Z"),
        Some(None)
    );
}

#[test]
fn reply_report_and_unmatched_close_lines_make_no_window() {
    let reply = json!({
        "v": 1, "at": "2026-11-01T10:05:00Z", "store": "/s", "worker_id": "w1",
        "event": "reply", "harness": "claude", "interrupt": false,
    });
    let windows = parse_windows(&jsonl(&[reply, close("w9", "2026-11-01T11:00:00Z")]));
    assert!(windows.is_empty());
}

#[test]
fn torn_and_unreadable_lines_are_skipped() {
    let text = format!(
        "{}not json\n{{\"at\":\"2026-11-01T10:0",
        jsonl(&[spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1"))])
    );
    assert_eq!(parse_windows(&text).len(), 1);
}

#[test]
fn a_spawn_with_an_unreadable_time_is_skipped() {
    let windows = parse_windows(&jsonl(&[spawn("w1", "yesterday", Some("GUZ-1"))]));
    assert!(windows.is_empty());
}

#[test]
fn overlapping_windows_pick_the_latest_start() {
    let windows = parse_windows(&jsonl(&[
        spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1")),
        spawn("w1", "2026-11-01T10:30:00Z", Some("GUZ-2")),
    ]));
    assert_eq!(
        ticket_at(&windows, "w1", "2026-11-01T10:45:00Z"),
        Some(Some("GUZ-2".into()))
    );
}

#[test]
fn instant_parses_rfc3339_with_an_offset_and_rejects_text() {
    assert_eq!(
        instant("2026-11-01T12:00:00+02:00"),
        instant("2026-11-01T10:00:00Z")
    );
    assert_eq!(instant("not a time"), None);
}

#[test]
fn window_position_names_the_index_of_the_joined_window() {
    let windows = parse_windows(&jsonl(&[
        spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1")),
        close("w1", "2026-11-01T11:00:00Z"),
        spawn("w1", "2026-11-02T10:00:00Z", Some("GUZ-2")),
    ]));
    assert_eq!(
        window_position(&windows, None, "w1", at("2026-11-02T10:30:00Z")),
        Some(1)
    );
    assert_eq!(
        window_position(&windows, None, "w1", at("2026-11-01T10:30:00Z")),
        Some(0)
    );
    assert_eq!(
        window_position(&windows, None, "w1", at("2026-10-31T00:00:00Z")),
        None
    );
}
