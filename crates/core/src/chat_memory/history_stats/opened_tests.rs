use super::opened::{count_opened, zoom_stats, Opened};
use super::test_rows::{call, history};
use serde_json::json;

fn search(hit_ids: serde_json::Value) -> String {
    call(json!({"hits": 2, "hit_ids": hit_ids}))
}

fn zoom(id: &str) -> String {
    call(json!({"tool": "zoom", "args": {"id": id}}))
}

fn opened(lines: &[String]) -> Opened {
    count_opened(&history(lines).calls)
}

#[test]
fn hits_covered_by_a_later_zoom_are_opened() {
    let found = opened(&[search(json!(["5", "9"])), zoom("8-11")]);
    assert_eq!(
        found,
        Opened {
            opened: 1,
            unopened: 1,
            unknown: 0
        }
    );
    let thread = opened(&[search(json!(["auth:5"])), zoom("auth:4-7")]);
    assert_eq!(thread.opened, 1);
}

#[test]
fn each_hit_id_counts_on_its_own() {
    let found = opened(&[search(json!(["5", "9"])), zoom("8-11")]);
    assert_eq!(
        found,
        Opened {
            opened: 1,
            unopened: 1,
            unknown: 0
        }
    );
    let all_open = opened(&[search(json!(["5", "9"])), zoom("4-11")]);
    assert_eq!((all_open.opened, all_open.unopened), (2, 0));
}

#[test]
fn hits_with_no_later_zoom_or_read_are_unopened() {
    let found = opened(&[zoom("5"), search(json!(["5"]))]);
    assert_eq!(
        found,
        Opened {
            opened: 0,
            unopened: 1,
            unknown: 0
        }
    );
    let other_turn = opened(&[
        search(json!(["5"])),
        call(json!({"tool": "zoom", "turn": "r1#2", "args": {"id": "5"}})),
    ]);
    assert_eq!(other_turn.unopened, 1);
    let other_scope = opened(&[search(json!(["auth:5"])), zoom("5")]);
    assert_eq!(other_scope.unopened, 1);
    let miss = opened(&[search(json!(["5"])), zoom("6-7")]);
    assert_eq!(miss.unopened, 1);
}

#[test]
fn rows_without_hit_ids_are_unknown() {
    let found = opened(&[
        call(json!({"hits": 3})),
        call(json!({"hits": 2, "hit_ids": ["5", "6"], "turn": null, "run": null, "source": "cli"})),
        call(json!({"hits": 0, "outcome": "empty"})),
        zoom("5"),
    ]);
    assert_eq!(
        found,
        Opened {
            opened: 0,
            unopened: 0,
            unknown: 5
        }
    );
}

#[test]
fn a_read_range_covers_a_hit_id() {
    let found = opened(&[
        search(json!(["5"])),
        call(json!({"tool": "read", "args": {"from": 3, "to": 6, "mode": "raw"}})),
    ]);
    assert_eq!(found.opened, 1);
    let malformed = opened(&[
        search(json!(["5"])),
        call(json!({"tool": "read", "outcome": "error", "args": {"id": "4-5"}})),
    ]);
    assert_eq!(malformed.opened, 1);
}

#[test]
fn zoom_depth_histogram() {
    let parsed = history(&[
        call(json!({"tool": "zoom", "depth": 0, "node_size": 1})),
        call(json!({"tool": "zoom", "depth": 2, "node_size": 4})),
        call(json!({"tool": "zoom", "depth": 4, "node_size": 16})),
        call(json!({"tool": "zoom", "depth": 7})),
        call(json!({"tool": "zoom"})),
        call(json!({"tool": "search", "depth": 1})),
    ]);
    let stats = zoom_stats(&parsed.calls);
    assert_eq!(stats.depth, [1, 0, 1, 0, 2]);
    assert_eq!((stats.size_count, stats.size_max), (3, 16));
}

#[test]
fn a_cli_row_counts_its_total_hits_as_unknown() {
    let ids: Vec<String> = (10..20).map(|n| n.to_string()).collect();
    let found = opened(&[call(json!({
        "hits": 30, "hit_ids": ids, "source": "cli", "turn": null, "run": null
    }))]);
    assert_eq!(found.unknown, 30);
}

#[test]
fn a_huge_hit_count_saturates_the_unknown_sum() {
    let huge = call(json!({"hits": 18446744073709551615u64, "source": "cli"}));
    let found = opened(&[huge.clone(), huge]);
    assert_eq!(found.unknown, usize::MAX);
}

#[test]
fn shown_ids_raise_a_total_hit_count_that_is_too_low() {
    let found = opened(&[call(json!({
        "hits": 0, "hit_ids": ["5"], "source": "cli", "turn": null, "run": null
    }))]);
    assert_eq!(found.unknown, 1);
}
