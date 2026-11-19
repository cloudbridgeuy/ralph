use super::old::{count_old, OldReach};
use super::test_rows::{call, history, turn};
use serde_json::json;

fn reach(lines: &[String]) -> OldReach {
    let parsed = history(lines);
    count_old(&parsed.calls, &parsed.turns)
}

fn zoom(id: &str) -> String {
    call(json!({"tool": "zoom", "args": {"id": id}}))
}

#[test]
fn old_reach_is_below_view_start() {
    let found = reach(&[
        zoom("2-3"),
        zoom("40-47"),
        call(json!({"tool": "read", "args": {"from": 1, "to": 9}})),
        call(json!({"oldest_hit": 0})),
        call(json!({"oldest_hit": 33})),
        turn(json!({"view_ids": ["32-39", "40-47", "48"]})),
    ]);
    assert_eq!(
        found,
        OldReach {
            old: 3,
            decided: 5,
            undecided: 0
        }
    );
}

#[test]
fn old_reach_uses_view_first_when_hashed() {
    let found = reach(&[
        zoom("10"),
        zoom("200"),
        turn(
            json!({"view_ids": null, "view_hash": "ab", "view_first": "100-103", "view_last": "250"}),
        ),
    ]);
    assert_eq!(
        found,
        OldReach {
            old: 1,
            decided: 2,
            undecided: 0
        }
    );
}

#[test]
fn old_reach_is_undecided_without_a_view() {
    let no_turn = call(json!({"tool": "zoom", "turn": null, "run": null, "args": {"id": "1"}}));
    let found = reach(&[no_turn, zoom("1"), turn(json!({"view_ids": []}))]);
    assert_eq!(
        found,
        OldReach {
            old: 0,
            decided: 0,
            undecided: 2
        }
    );
    let no_row = reach(&[zoom("1")]);
    assert_eq!(no_row.undecided, 1);
}

#[test]
fn an_empty_view_uses_the_latest_earlier_view_of_the_run() {
    let found = reach(&[
        turn(json!({"turn": "r1#1", "view_ids": ["50-51"]})),
        call(json!({"tool": "zoom", "turn": "r1#2", "args": {"id": "5"}})),
        turn(json!({"turn": "r1#2", "view_ids": []})),
        call(json!({"tool": "zoom", "turn": "r2#1", "run": "r2", "args": {"id": "5"}})),
        turn(json!({"turn": "r2#1", "run": "r2", "view_ids": []})),
    ]);
    assert_eq!(
        found,
        OldReach {
            old: 1,
            decided: 1,
            undecided: 1
        }
    );
}

#[test]
fn other_scopes_are_not_reaches() {
    let found = reach(&[
        zoom("auth:2"),
        turn(json!({"view_ids": ["main:30", "auth:1", "32"]})),
    ]);
    assert_eq!(found, OldReach::default());
    let thread = reach(&[
        call(json!({"tool": "zoom", "scope": "auth", "args": {"id": "2"}})),
        turn(json!({"scope": "auth", "view_ids": ["auth:9", "main:1", "12"]})),
    ]);
    assert_eq!(
        thread,
        OldReach {
            old: 1,
            decided: 1,
            undecided: 0
        }
    );
}
