use super::chains::{build_chains, Chains};
use super::config::StatsConfig;
use super::test_rows::{at, call, history};
use serde_json::{json, Value};

fn weak(second: u32, extra: Value) -> String {
    let mut row = json!({"at": at(second), "outcome": "empty", "hits": 0});
    if let (Some(map), Some(more)) = (row.as_object_mut(), extra.as_object()) {
        map.extend(more.clone());
    }
    call(row)
}

fn chains(lines: &[String]) -> Chains {
    build_chains(&history(lines).calls, &StatsConfig::default())
}

#[test]
fn three_empty_searches_in_a_turn_make_a_chain() {
    let found = chains(&[weak(0, json!({})), weak(10, json!({})), weak(20, json!({}))]);
    assert_eq!(
        found,
        Chains {
            chains: 1,
            calls_in_chains: 3
        }
    );
}

#[test]
fn two_weak_calls_are_too_few() {
    let found = chains(&[weak(0, json!({})), weak(10, json!({}))]);
    assert_eq!(found, Chains::default());
}

#[test]
fn a_gap_over_the_threshold_splits_a_chain() {
    let found = chains(&[
        weak(0, json!({})),
        weak(10, json!({})),
        weak(20, json!({})),
        weak(300, json!({})),
        weak(310, json!({})),
    ]);
    assert_eq!(
        found,
        Chains {
            chains: 1,
            calls_in_chains: 3
        }
    );
    let both = chains(&[
        weak(0, json!({})),
        weak(10, json!({})),
        weak(20, json!({})),
        weak(200, json!({})),
        weak(210, json!({})),
        weak(220, json!({})),
    ]);
    assert_eq!(
        both,
        Chains {
            chains: 2,
            calls_in_chains: 6
        }
    );
}

#[test]
fn a_many_hit_call_breaks_a_chain() {
    let found = chains(&[
        weak(0, json!({})),
        weak(10, json!({})),
        weak(20, json!({"outcome": "ok", "hits": 9})),
        weak(30, json!({})),
        weak(40, json!({})),
    ]);
    assert_eq!(found, Chains::default());
    let few = chains(&[
        weak(0, json!({"outcome": "ok", "hits": 2})),
        weak(10, json!({"outcome": "ok", "hits": 1})),
        weak(20, json!({"tool": "recall"})),
    ]);
    assert_eq!(
        few,
        Chains {
            chains: 1,
            calls_in_chains: 3
        }
    );
}

#[test]
fn error_rows_and_unknown_hits_break_a_chain() {
    let errored = chains(&[
        weak(0, json!({})),
        weak(10, json!({"outcome": "error"})),
        weak(20, json!({})),
    ]);
    assert_eq!(errored, Chains::default());
    let unknown = chains(&[
        weak(0, json!({})),
        weak(10, json!({"outcome": "ok", "hits": null})),
        weak(20, json!({})),
    ]);
    assert_eq!(unknown, Chains::default());
}

#[test]
fn cli_rows_never_chain() {
    let no_turn = |second| weak(second, json!({"source": "cli", "turn": null, "run": null}));
    let found = chains(&[no_turn(0), no_turn(5), no_turn(10)]);
    assert_eq!(found, Chains::default());
    let cli_with_turn = |second| weak(second, json!({"source": "cli"}));
    let found = chains(&[cli_with_turn(0), cli_with_turn(5), cli_with_turn(10)]);
    assert_eq!(found, Chains::default());
}

#[test]
fn chains_group_by_turn_and_ignore_other_tools() {
    let found = chains(&[
        weak(0, json!({"turn": "r1#1"})),
        weak(5, json!({"turn": "r1#2"})),
        weak(10, json!({"turn": "r1#1"})),
        weak(12, json!({"turn": "r1#1", "tool": "zoom"})),
        weak(15, json!({"turn": "r1#2"})),
        weak(20, json!({"turn": "r1#1"})),
    ]);
    assert_eq!(
        found,
        Chains {
            chains: 1,
            calls_in_chains: 3
        }
    );
}

#[test]
fn tag_only_listings_with_few_hits_count_as_weak_calls() {
    let listing = |second, hits| {
        weak(
            second,
            json!({
                "outcome": "ok",
                "hits": hits,
                "args": {"query": "", "tag": "rule", "limit": 20, "has_cursor": false}
            }),
        )
    };
    let found = chains(&[listing(0, 2), listing(10, 1), listing(20, 2)]);
    assert_eq!(
        found,
        Chains {
            chains: 1,
            calls_in_chains: 3
        }
    );
    assert_eq!(
        chains(&[listing(0, 2), listing(10, 1), listing(20, 5)]),
        Chains::default()
    );
}
