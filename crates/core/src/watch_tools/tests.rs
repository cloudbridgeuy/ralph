use super::*;
use crate::watch::{WatchKind, WatchSpec};

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
}

fn saved(id: &str, kind: WatchKind, timeout: Option<u64>) -> SavedWatch {
    SavedWatch::new(
        &WatchSpec {
            id: id.to_string(),
            command: format!("cmd {id}"),
            follow_up: Some("f".to_string()),
            kind,
            timeout: timeout.map(Duration::from_secs),
            limit: timeout.map(Duration::from_secs),
        },
        at(0),
    )
}

fn request(id: &str, timeout: Option<u64>) -> WatchRequest {
    WatchRequest {
        id: id.to_string(),
        command: format!("cmd {id}"),
        follow_up: "f".to_string(),
        persistent: false,
        timeout_secs: timeout,
    }
}

#[test]
fn running_wins_over_saved_and_queued_of_the_same_id() {
    let run = [saved("watch:a", WatchKind::Persistent, Some(600))];
    let rows = merge_rows(
        &[request("watch:a", None)],
        &run,
        &[saved("watch:a", WatchKind::Persistent, Some(600))],
        at(60),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, WatchState::Running);
    assert_eq!(rows[0].left, Some(Duration::from_secs(540)));
}

#[test]
fn rows_hold_all_three_states_sorted_by_id() {
    let rows = merge_rows(
        &[request("watch:q", Some(30))],
        &[saved("watch:r", WatchKind::Normal, None)],
        &[saved("watch:p", WatchKind::Persistent, Some(60))],
        at(0),
    );
    let seen: Vec<(&str, WatchState)> = rows.iter().map(|r| (r.id.as_str(), r.state)).collect();
    assert_eq!(
        seen,
        [
            ("watch:p", WatchState::Saved),
            ("watch:q", WatchState::Queued),
            ("watch:r", WatchState::Running)
        ]
    );
}

#[test]
fn no_rows_say_no_active_watches() {
    assert_eq!(format_rows(&[]), "No active watches.");
}

#[test]
fn a_running_row_shows_every_field() {
    let rows = merge_rows(
        &[],
        &[saved("watch:a", WatchKind::Persistent, Some(7200))],
        &[],
        at(60),
    );
    assert_eq!(
        format_rows(&rows),
        "watch:a running persistent 1h 7200s 2023-11-14T22:13:20+00:00 cmd watch:a"
    );
}

#[test]
fn a_queued_row_has_no_start_or_time_left() {
    let rows = merge_rows(&[request("watch:q", None)], &[], &[], at(0));
    assert_eq!(
        format_rows(&rows),
        "watch:q queued normal - default - cmd watch:q"
    );
}

#[test]
fn a_long_command_is_cut_and_flattened() {
    assert_eq!(short_command("a\n  b"), "a b");
    let cut = short_command(&"x".repeat(200));
    assert_eq!(cut.chars().count(), 80);
    assert!(cut.ends_with('…'));
}

#[test]
fn the_definition_takes_no_arguments() {
    let def = watch_list_definition();
    assert_eq!(def["name"], "watch_list");
    assert!(def["inputSchema"]["properties"]
        .as_object()
        .unwrap()
        .is_empty());
}
