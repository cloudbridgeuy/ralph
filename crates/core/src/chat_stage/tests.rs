use super::*;
use crate::chat_status::{ScheduleState, WatchState, WorkerState};
use serde_json::json;

fn item(id: &str, kind: Kind, turn: usize) -> Item {
    Item {
        id: id.to_string(),
        kind,
        reference: None,
        text: "approve merge".to_string(),
        created: "2026-01-01T00:00:00Z".to_string(),
        turn,
    }
}

fn worker(id: &str, state: &str) -> WorkerState {
    WorkerState {
        id: id.to_string(),
        state: state.to_string(),
    }
}

#[test]
fn nothing_open_renders_nothing() {
    assert_eq!(render(&[], &Status::default()), "");
}

#[test]
fn empty_categories_are_omitted() {
    let status = Status {
        unread: 2,
        ..Status::default()
    };
    assert_eq!(render(&[], &status), "Stage:\ninbox: 2 unread");
}

#[test]
fn model_items_take_one_line_each_with_the_ref_when_given() {
    let mut first = item("s2", Kind::Decision, 0);
    first.reference = Some("GUZ-1805".to_string());
    let rendered = render(&[first, item("s3", Kind::Owed, 0)], &Status::default());
    assert_eq!(
        rendered,
        "Stage:\ns2 decision GUZ-1805: approve merge\ns3 owed approve merge"
    );
}

#[test]
fn workers_needing_the_user_get_their_own_line() {
    let status = Status {
        workers: vec![
            worker("w1", "question"),
            worker("w2", "running"),
            worker("w3", "blocked"),
            worker("w4", "permission"),
            worker("w5", "done"),
        ],
        ..Status::default()
    };
    assert_eq!(
        render(&[], &status),
        "Stage:\nneeds you: w1 question, w3 blocked, w4 permission\nworkers: w2, w5"
    );
}

#[test]
fn watches_show_no_countdown_and_schedules_show_their_time() {
    let status = Status {
        watches: vec![WatchState {
            id: "wa1".to_string(),
            left: Some("4m".to_string()),
        }],
        schedule: vec![ScheduleState {
            id: "sc1".to_string(),
            time: "14:30".to_string(),
        }],
        ..Status::default()
    };
    assert_eq!(
        render(&[], &status),
        "Stage:\nwatches: wa1\nschedule: sc1 14:30"
    );
}

#[test]
fn followup_and_owed_expire_by_log_distance_and_decision_never() {
    let stage = Stage {
        items: vec![
            item("s1", Kind::Followup, 0),
            item("s2", Kind::Owed, 0),
            item("s3", Kind::Decision, 0),
            item("s4", Kind::Followup, 41),
        ],
        last: 4,
    };
    let (kept, gone) = expire(stage, 100);
    let ids = |items: &[Item]| items.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&kept.items), ["s3", "s4"]);
    assert_eq!(ids(&gone), ["s1", "s2"]);
    assert_eq!(kept.last, 4);
}

#[test]
fn an_item_expires_exactly_at_the_distance() {
    let stage = Stage {
        items: vec![item("s1", Kind::Owed, 10)],
        last: 1,
    };
    assert_eq!(expire(stage.clone(), 69).1.len(), 0);
    assert_eq!(expire(stage, 70).1.len(), 1);
}

#[test]
fn the_stage_is_due_on_first_send_on_change_and_at_the_repeat() {
    let mark = StageMark::sent("Stage:\na");
    assert!(stage_due(None, "Stage:\na"));
    assert!(!stage_due(None, ""));
    assert!(stage_due(Some(&mark), "Stage:\nb"));
    assert!(!stage_due(Some(&mark), "Stage:\na"));
    let waiting = (0..STAGE_REPEAT_TURNS).fold(mark, |mark, _| mark.skipped());
    assert!(stage_due(Some(&waiting), "Stage:\na"));
}

#[test]
fn an_emptied_stage_is_due_once() {
    let mark = StageMark::sent("Stage:\na");
    assert!(stage_due(Some(&mark), ""));
    assert_eq!(stage_text(""), "Stage: empty");
    let after = StageMark::sent("");
    let later = (0..STAGE_REPEAT_TURNS).fold(after, |mark, _| mark.skipped());
    assert!(!stage_due(Some(&later), ""));
}

#[test]
fn add_arguments_are_checked() {
    let ok = parse_add(&json!({"kind": "owed", "text": " reply to w1 ", "ref": "w1"})).unwrap();
    assert_eq!(ok.kind, Kind::Owed);
    assert_eq!(ok.text, "reply to w1");
    assert_eq!(ok.reference.as_deref(), Some("w1"));
    assert!(parse_add(&json!({"kind": "later", "text": "x"})).is_err());
    assert!(parse_add(&json!({"kind": "owed", "text": " "})).is_err());
    assert!(parse_add(&json!({"kind": "owed", "text": "a\nb"})).is_err());
    assert!(parse_add(&json!({"kind": "owed", "text": "x".repeat(5000)})).is_ok());
    assert!(parse_add(&json!({"kind": "owed", "text": "x", "ref": 3})).is_err());
}

#[test]
fn done_needs_an_id() {
    let closing = parse_done(&json!({"id": "s4", "note": "approved"})).unwrap();
    assert_eq!(
        (closing.id.as_str(), closing.note.as_deref()),
        ("s4", Some("approved"))
    );
    assert_eq!(parse_done(&json!({"id": "s4"})).unwrap().note, None);
    assert!(parse_done(&json!({})).is_err());
    assert!(parse_done(&json!({"id": "s4", "note": "a\nb"})).is_err());
}

fn new_item(text: &str) -> NewItem {
    NewItem {
        kind: Kind::Owed,
        reference: None,
        text: text.to_string(),
    }
}

#[test]
fn ids_grow_and_are_never_reused_after_a_close() {
    let (stage, first) = add(Stage::default(), new_item("a"), "t", 3);
    let (stage, second) = add(stage, new_item("b"), "t", 4);
    assert_eq!((first.id.as_str(), second.id.as_str()), ("s1", "s2"));
    assert_eq!(second.turn, 4);
    let (stage, _) = close(stage, "s2").unwrap();
    let (_, third) = add(stage, new_item("c"), "t", 5);
    assert_eq!(third.id, "s3");
}

#[test]
fn closing_removes_the_item_and_an_unknown_id_is_an_error() {
    let (stage, _) = add(Stage::default(), new_item("a"), "t", 0);
    assert!(close(stage.clone(), "s9").is_err());
    let (left, gone) = close(stage, "s1").unwrap();
    assert_eq!(gone.text, "a");
    assert!(left.items.is_empty());
    assert_eq!(left.last, 1);
}

#[test]
fn a_launch_or_a_send_resets_the_mark_and_a_skip_counts_a_turn() {
    let held = StageMark::sent("Stage:\na");
    assert_eq!(
        next_mark(Some(&held), false, false, "Stage:\na").turns_since,
        1
    );
    assert_eq!(
        next_mark(Some(&held), true, false, "Stage:\nb"),
        StageMark::sent("Stage:\nb")
    );
    assert_eq!(next_mark(Some(&held), false, true, ""), StageMark::sent(""));
    assert_eq!(next_mark(None, true, true, "x"), StageMark::sent("x"));
}
