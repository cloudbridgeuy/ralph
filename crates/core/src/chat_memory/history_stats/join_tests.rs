use super::config::StatsConfig;
use super::join::{join_turns, Join};
use super::parse::{parse_timing, TimingParsed};
use super::test_rows::{at, history, turn};
use serde_json::json;

fn timing(rows: &[(&str, u32)]) -> TimingParsed {
    let lines: Vec<String> = rows
        .iter()
        .map(|(run, second)| {
            json!({"v": 1, "at": at(*second), "event": "turn", "run": run, "turn_ms": 100, "ok": true})
                .to_string()
        })
        .collect();
    parse_timing(&lines.join("\n"))
}

fn hist(rows: &[(&str, u32, u64)]) -> Vec<super::parse::TurnRow> {
    let lines: Vec<String> = rows
        .iter()
        .map(|(run, second, calls)| {
            turn(json!({"run": run, "at": at(*second), "history_calls": calls}))
        })
        .collect();
    history(&lines).turns
}

#[test]
fn timing_joins_history_turn_by_run_and_nearest_time() {
    let turns = hist(&[("r1", 10, 2), ("r1", 20, 1), ("r2", 10, 4)]);
    let timed = timing(&[("r1", 21), ("r1", 10), ("r2", 11)]);
    let found = join_turns(&turns, &timed.turns, &StatsConfig::default());
    assert_eq!(
        found,
        Join {
            timing_present: true,
            total_turns: 3,
            joined: 3,
            no_history_calls: 0,
            unjoined_timing: 0,
            history_without_timing: 0,
        }
    );
}

#[test]
fn turns_with_no_history_call_are_counted() {
    let turns = hist(&[("r1", 10, 0), ("r1", 20, 3), ("r1", 30, 0)]);
    let timed = timing(&[("r1", 10), ("r1", 20), ("r1", 30)]);
    let found = join_turns(&turns, &timed.turns, &StatsConfig::default());
    assert_eq!((found.joined, found.no_history_calls), (3, 2));
}

#[test]
fn unmatched_timing_rows_are_reported() {
    let turns = hist(&[("r1", 10, 1), ("r1", 50, 1)]);
    let timed = timing(&[("r1", 10), ("r1", 30), ("r2", 50), ("r1", 51)]);
    let found = join_turns(&turns, &timed.turns, &StatsConfig::default());
    assert_eq!(found.total_turns, 4);
    assert_eq!(found.joined, 2);
    assert_eq!(found.unjoined_timing, 2);
    assert_eq!(found.history_without_timing, 0);
    let far = join_turns(
        &hist(&[("r1", 10, 1)]),
        &timing(&[("r1", 14)]).turns,
        &StatsConfig::default(),
    );
    assert_eq!(
        (far.joined, far.history_without_timing, far.unjoined_timing),
        (0, 1, 1)
    );
}

#[test]
fn a_timing_row_joins_only_one_history_row() {
    let turns = hist(&[("r1", 10, 1), ("r1", 11, 1)]);
    let timed = timing(&[("r1", 10)]);
    let found = join_turns(&turns, &timed.turns, &StatsConfig::default());
    assert_eq!((found.joined, found.history_without_timing), (1, 1));
}

#[test]
fn history_rows_are_the_denominator_when_timing_is_empty() {
    let turns = hist(&[("r1", 10, 0), ("r1", 20, 2)]);
    let found = join_turns(&turns, &[], &StatsConfig::default());
    assert_eq!(
        found,
        Join {
            timing_present: false,
            total_turns: 2,
            joined: 0,
            no_history_calls: 1,
            unjoined_timing: 0,
            history_without_timing: 0,
        }
    );
}
