use super::config::StatsConfig;
use super::render::render_stats;
use super::stats::build_stats;
use super::test_rows::{at, call, turn};
use serde_json::json;

fn history_text() -> String {
    [
        call(json!({"at": at(0), "hits": 2, "hit_ids": ["5", "9"]})),
        call(json!({"at": at(5), "tool": "zoom", "args": {"id": "5"}, "depth": 0, "node_size": 1})),
        call(json!({"at": at(10), "outcome": "empty", "hits": 0})),
        turn(json!({"at": at(20), "history_calls": 3, "view_ids": ["8-11"]})),
        call(json!({
            "at": at(30), "source": "cli", "tool": "read", "outcome": "error",
            "turn": null, "run": null, "args": {"id": "1"}
        })),
        "garbage".to_string(),
    ]
    .join("\n")
}

fn timing_text() -> String {
    [
        json!({"v": 1, "at": at(20), "event": "turn", "run": "r1", "turn_ms": 50, "ok": true}),
        json!({"v": 1, "at": at(40), "event": "turn", "run": "r1", "turn_ms": 60, "ok": true}),
    ]
    .map(|row| row.to_string())
    .join("\n")
}

#[test]
fn render_prints_sections_in_order() {
    let stats = build_stats(&history_text(), &timing_text(), &StatsConfig::default());
    let text = render_stats(&stats);
    let titles = [
        "history stats",
        "calls",
        "rates",
        "chains",
        "unopened hits",
        "no-history turns",
        "zoom depth",
        "old reach",
    ];
    let places: Vec<_> = titles
        .iter()
        .map(|title| text.lines().position(|line| line == *title))
        .collect();
    assert!(places.iter().all(Option::is_some), "{text}");
    assert!(places.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
}

#[test]
fn empty_input_prints_no_rows() {
    let stats = build_stats("", "", &StatsConfig::default());
    assert_eq!(render_stats(&stats), "no history rows\n");
}

#[test]
fn build_and_render_a_small_fixture() {
    let stats = build_stats(&history_text(), &timing_text(), &StatsConfig::default());
    let expected = "\
history stats
call rows: 4, turn rows: 1, skipped rows: 1

calls
tool     total   mcp slash   cli other
read         1     0     0     1     0
zoom         1     1     0     0     0
recall       0     0     0     0     0
search       2     2     0     0     0

rates
tool     empty error
read        0%  100%
zoom        0%    0%
recall      0%    0%
search     50%    0%

chains
chains: 0
calls in chains: 0

unopened hits
opened: 1
unopened: 0
unknown: 0

no-history turns
turns: 2
joined: 1
with no history call: 0
timing rows with no history row: 1
history rows with no timing row: 0

zoom depth
0: 1
1: 0
2: 0
3: 0
4+: 0
node size: count 1, max 1

old reach
old: 1
decided: 1
undecided: 1
";
    assert_eq!(render_stats(&stats), expected);
}

#[test]
fn render_says_so_when_timing_is_missing() {
    let text = render_stats(&build_stats(&history_text(), "", &StatsConfig::default()));
    assert!(
        text.contains("turns: 1 (no timing log)\njoined: n/a\n"),
        "{text}"
    );
}
