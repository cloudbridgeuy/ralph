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

fn rows() -> Vec<WatchRow> {
    merge_rows(
        &[request("watch:q", None)],
        &[
            saved("watch:r", WatchKind::Persistent, Some(60)),
            saved("schedule:nightly", WatchKind::Normal, None),
            saved("watch:send-it-1", WatchKind::Normal, None),
        ],
        &[saved("watch:p", WatchKind::Persistent, Some(60))],
        at(0),
    )
}

fn ids(args: Value) -> Result<StopTarget, String> {
    parse_stop_args(&args)
}

#[test]
fn stop_ids_get_the_watch_prefix_and_lose_duplicates() {
    assert_eq!(
        ids(json!({"ids": ["w1", "watch:w1", "ci"]})),
        Ok(StopTarget::Ids(vec![
            "watch:w1".to_string(),
            "watch:ci".to_string()
        ]))
    );
}

#[test]
fn stop_all_is_a_true_flag() {
    assert_eq!(ids(json!({"all": true})), Ok(StopTarget::All));
}

#[test]
fn stop_needs_exactly_one_of_ids_or_all() {
    for bad in [
        json!({}),
        json!({"all": false}),
        json!({"ids": []}),
        json!({"ids": ["a"], "all": true}),
        json!({"ids": "a"}),
        json!({"ids": [1]}),
        json!({"ids": ["a b"]}),
        json!({"ids": ["a/b"]}),
        json!({"all": "yes"}),
    ] {
        assert!(ids(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn stopping_ids_picks_the_action_by_state() {
    let target = StopTarget::Ids(
        ["watch:r", "watch:q", "watch:p", "watch:nope"]
            .map(String::from)
            .to_vec(),
    );
    assert_eq!(
        plan_stop(&target, &rows()),
        [
            StopStep::Stop("watch:r".to_string()),
            StopStep::Dequeue("watch:q".to_string()),
            StopStep::Stop("watch:p".to_string()),
            StopStep::Unknown("watch:nope".to_string()),
        ]
    );
}

#[test]
fn stopping_all_skips_schedule_and_send_it_check_watches() {
    let steps = plan_stop(&StopTarget::All, &rows());
    assert_eq!(
        steps,
        [
            StopStep::Skip {
                id: "schedule:nightly".to_string(),
                reason: SKIP_SCHEDULE
            },
            StopStep::Stop("watch:p".to_string()),
            StopStep::Dequeue("watch:q".to_string()),
            StopStep::Stop("watch:r".to_string()),
            StopStep::Skip {
                id: "watch:send-it-1".to_string(),
                reason: SKIP_SEND_IT
            },
        ]
    );
}

#[test]
fn stopping_all_with_no_watches_plans_nothing() {
    assert!(plan_stop(&StopTarget::All, &[]).is_empty());
    assert_eq!(stop_report(&[]), "No active watches.");
}

#[test]
fn the_report_joins_one_line_per_step() {
    assert_eq!(stop_report(&["a".to_string(), "b".to_string()]), "a\nb");
    assert_eq!(
        dequeued_message("watch:q"),
        "Removed queued watch watch:q before it started."
    );
    assert_eq!(
        skipped_message("schedule:x", SKIP_SCHEDULE),
        "Skipped schedule:x: it is a scheduled task; use /schedule cancel."
    );
}

#[test]
fn the_stop_definition_names_ids_and_all() {
    let def = watch_stop_definition();
    assert_eq!(def["name"], "watch_stop");
    assert!(def["description"]
        .as_str()
        .unwrap()
        .contains("latest message"));
    let props = def["inputSchema"]["properties"].as_object().unwrap();
    assert!(props.contains_key("ids") && props.contains_key("all"));
}
