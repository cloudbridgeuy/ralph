use super::{
    idle_periods, miss_causes, outcome_counts, reset_writes, IdlePeriod, MissCauses, PeriodOutcome,
};
use crate::chat_process::ResetReason;
use crate::worker::TicketKeys;
use crate::worker_costs::usage_report::cache_view::ProcessStart;
use crate::worker_costs::usage_report::{parse, Entry};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

fn at(minute: u32) -> String {
    format!("2026-11-10T{:02}:{:02}:00Z", 10 + minute / 60, minute % 60)
}

fn instant(minute: u32) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&at(minute))
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

fn turn(run: &str, minute: u32, process_turn: u64, tokens: (u64, u64, u64)) -> Value {
    let (input, write, read) = tokens;
    json!({
        "at": at(minute),
        "kind": "turn",
        "run": run,
        "cost_usd": 0.5,
        "context_tokens": input + write + read,
        "input_tokens": input,
        "cache_creation_input_tokens": write,
        "cache_read_input_tokens": read,
        "output_tokens": 5,
        "process_turn": process_turn,
        "model": "claude-opus-5-5",
        "ledger_v": 2,
    })
}

fn ping(run: &str, minute: u32, cost: f64) -> Value {
    json!({
        "at": at(minute),
        "kind": "ping",
        "run": run,
        "cost_usd": cost,
        "input_tokens": 3,
        "output_tokens": 1,
        "model": "claude-opus-5-5",
        "ledger_v": 2,
    })
}

fn entries(rows: &[Value]) -> Vec<Entry> {
    let text: String = rows.iter().map(|row| format!("{row}\n")).collect();
    parse(&text, &[], &TicketKeys::default()).entries
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

#[test]
fn a_ping_then_a_cache_read_turn_is_warm() {
    let rows = [
        turn("r", 0, 1, (4, 18_000, 0)),
        ping("r", 4, 0.02),
        turn("r", 8, 5, (4, 200, 30_000)),
    ];
    let found = idle_periods(&entries(&rows));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].outcome, PeriodOutcome::Warm);
    assert_eq!(found[0].pings, 1);
    assert_eq!(found[0].last_turn_at, instant(0));
    assert_eq!(found[0].last_ping_at, instant(4));
    assert_eq!(found[0].next_turn_at, instant(8));
    assert_eq!(found[0].context_tokens, Some(18_004));
}

#[test]
fn a_next_turn_that_starts_a_process_is_cold() {
    let rows = [
        turn("r", 0, 3, (4, 200, 30_000)),
        ping("r", 4, 0.02),
        turn("r", 50, 1, (4, 200, 30_000)),
    ];
    let found = idle_periods(&entries(&rows));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].outcome, PeriodOutcome::Cold);
}

#[test]
fn a_write_heavy_next_turn_is_cold() {
    let rows = [
        turn("r", 0, 3, (4, 200, 30_000)),
        ping("r", 4, 0.02),
        turn("r", 50, 5, (4, 18_000, 0)),
    ];
    let found = idle_periods(&entries(&rows));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].outcome, PeriodOutcome::Cold);
}

#[test]
fn pings_at_the_end_of_a_run_have_no_next_turn() {
    let rows = [
        turn("r", 0, 3, (4, 200, 30_000)),
        ping("r", 4, 0.02),
        ping("r", 8, 0.02),
    ];
    let found = idle_periods(&entries(&rows));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].outcome, PeriodOutcome::NoNextTurn);
    assert_eq!(found[0].next_turn_at, None);
    assert_eq!(found[0].last_ping_at, instant(8));
}

#[test]
fn four_pings_count_and_sum_their_cost() {
    let rows = [
        turn("r", 0, 3, (4, 200, 30_000)),
        ping("r", 4, 0.01),
        ping("r", 8, 0.02),
        ping("r", 12, 0.03),
        ping("r", 16, 0.04),
        turn("r", 20, 4, (4, 200, 30_000)),
    ];
    let found = idle_periods(&entries(&rows));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].pings, 4);
    assert!(close(found[0].ping_cost, 0.10));
    assert_eq!(found[0].last_ping_at, instant(16));
}

#[test]
fn old_style_running_totals_sum_through_normalize() {
    let old = |kind: &str, minute: u32, cost: f64| {
        json!({
            "at": at(minute),
            "kind": kind,
            "run": "r",
            "cost_usd": cost,
            "context_tokens": 100,
            "input_tokens": 4,
            "cache_read_input_tokens": 100,
            "output_tokens": 1,
            "model": "claude-opus-5-5",
        })
    };
    let rows = [
        old("turn", 0, 0.28),
        old("ping", 4, 0.30),
        old("ping", 8, 0.32),
        old("turn", 12, 0.60),
    ];
    let found = idle_periods(&entries(&rows));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].pings, 2);
    assert!(close(found[0].ping_cost, 0.04));
}

#[test]
fn request_at_wins_over_at_for_the_last_turn() {
    let mut first = turn("r", 5, 3, (4, 200, 30_000));
    first["request_at"] = json!(at(1));
    let rows = [first, ping("r", 9, 0.02)];
    let found = idle_periods(&entries(&rows));
    assert_eq!(found[0].last_turn_at, instant(1));
}

#[test]
fn interleaved_runs_make_separate_periods() {
    let rows = [
        turn("a", 0, 3, (4, 200, 30_000)),
        turn("b", 1, 3, (4, 200, 30_000)),
        ping("a", 4, 0.02),
        ping("b", 5, 0.05),
        ping("b", 6, 0.05),
        turn("a", 8, 4, (4, 200, 30_000)),
    ];
    let found = idle_periods(&entries(&rows));
    assert_eq!(found.len(), 2);
    assert_eq!((found[0].pings, found[0].outcome), (1, PeriodOutcome::Warm));
    assert_eq!(
        (found[1].pings, found[1].outcome),
        (2, PeriodOutcome::NoNextTurn)
    );
}

#[test]
fn pings_before_any_turn_form_no_period() {
    let rows = [
        ping("r", 0, 0.02),
        ping("r", 4, 0.02),
        turn("r", 8, 1, (4, 18_000, 0)),
    ];
    assert!(idle_periods(&entries(&rows)).is_empty());
}

#[test]
fn a_turn_followed_by_a_turn_makes_no_period() {
    let rows = [
        turn("r", 0, 3, (4, 200, 30_000)),
        turn("r", 4, 4, (4, 200, 30_000)),
    ];
    assert!(idle_periods(&entries(&rows)).is_empty());
}

#[test]
fn outcome_counts_total_periods_pings_and_cost() {
    let period = |pings, ping_cost, outcome| IdlePeriod {
        pings,
        ping_cost,
        last_turn_at: None,
        last_ping_at: None,
        context_tokens: None,
        model: None,
        next_turn_at: None,
        outcome,
    };
    let counts = outcome_counts(&[
        period(2, 0.5, PeriodOutcome::Warm),
        period(1, 0.25, PeriodOutcome::Cold),
        period(3, 0.25, PeriodOutcome::Warm),
        period(1, 0.0, PeriodOutcome::NoNextTurn),
    ]);
    assert_eq!((counts.periods, counts.pings), (4, 7));
    assert!(close(counts.ping_cost, 1.0));
    assert_eq!((counts.warm, counts.cold, counts.no_next), (2, 1, 1));
}

fn reset_turn(
    run: &str,
    minute: u32,
    reason: Option<&str>,
    write: u64,
    context: Option<u64>,
) -> Value {
    let mut row = turn(run, minute, 1, (4, write, 0));
    row["context_tokens"] = json!(context);
    if let Some(reason) = reason {
        row["reset"] = json!(reason);
    }
    row
}

#[test]
fn reset_writes_group_by_start_in_fixed_order() {
    let rows = [
        reset_turn("a", 0, None, 10_000, Some(20_000)),
        reset_turn("b", 1, Some("restart"), 5_000, Some(10_000)),
        reset_turn("c", 2, Some("tokens"), 15_000, Some(20_000)),
        reset_turn("d", 3, Some("turns"), 8_000, Some(10_000)),
    ];
    let groups = reset_writes(&entries(&rows));
    let starts: Vec<ProcessStart> = groups.iter().map(|group| group.start).collect();
    assert_eq!(
        starts,
        vec![
            ProcessStart::Reset(ResetReason::Tokens),
            ProcessStart::Reset(ResetReason::Turns),
            ProcessStart::Reset(ResetReason::Restart),
            ProcessStart::NewRun,
        ]
    );
    let ratios: Vec<Option<f64>> = groups.iter().map(|group| group.mean_ratio).collect();
    assert_eq!(ratios, vec![Some(0.75), Some(0.8), Some(0.5), Some(0.5)]);
}

#[test]
fn reset_writes_skip_rows_without_context_and_average_the_rest() {
    let rows = [
        reset_turn("a", 0, Some("tokens"), 10_000, Some(20_000)),
        reset_turn("b", 1, Some("tokens"), 20_000, Some(20_000)),
        reset_turn("c", 2, Some("tokens"), 4_000, None),
    ];
    let groups = reset_writes(&entries(&rows));
    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert_eq!(group.turns, 3);
    assert!(close(group.mean_ratio.unwrap(), 0.75));
    assert!(close(group.mean_write_tokens.unwrap(), 34_000.0 / 3.0));
}

#[test]
fn reset_writes_without_any_ratio_has_no_mean_ratio() {
    let rows = [reset_turn("a", 0, Some("turns"), 4_000, None)];
    let groups = reset_writes(&entries(&rows));
    assert_eq!(groups[0].mean_ratio, None);
    assert_eq!(groups[0].turns, 1);
}

#[test]
fn reset_writes_ignore_later_turns() {
    let rows = [turn("a", 0, 2, (4, 100, 0))];
    assert!(reset_writes(&entries(&rows)).is_empty());
}

fn misses(rows: &[Value]) -> MissCauses {
    miss_causes(&entries(rows))
}

#[test]
fn a_reset_turn_write_is_a_reset_miss() {
    let rows = [reset_turn("a", 0, Some("tokens"), 10_000, Some(10_004))];
    let causes = misses(&rows);
    assert_eq!((causes.misses, causes.reset, causes.turns), (1, 1, 1));
}

#[test]
fn a_read_heavy_reset_turn_is_still_a_reset_miss() {
    let mut row = turn("a", 0, 1, (4, 100, 50_000));
    row["reset"] = json!("tokens");
    let causes = misses(&[row]);
    assert_eq!(
        (causes.misses, causes.reset, causes.new_run, causes.turns),
        (1, 1, 0, 1)
    );
}

#[test]
fn a_read_heavy_first_launch_is_not_a_miss() {
    let rows = [turn("a", 0, 1, (4, 100, 50_000))];
    let causes = misses(&rows);
    assert_eq!((causes.misses, causes.new_run, causes.turns), (0, 0, 1));
}

#[test]
fn a_new_run_first_turn_write_has_its_own_bucket() {
    let rows = [reset_turn("a", 0, None, 10_000, Some(10_004))];
    let causes = misses(&rows);
    assert_eq!((causes.misses, causes.new_run, causes.reset), (1, 1, 0));
}

#[test]
fn a_write_heavy_turn_after_more_than_an_hour_is_ttl() {
    let rows = [
        turn("a", 0, 1, (4, 100, 18_000)),
        turn("a", 61, 2, (4, 18_000, 0)),
    ];
    let causes = misses(&rows);
    assert_eq!((causes.misses, causes.ttl, causes.unexplained), (1, 1, 0));
}

#[test]
fn a_write_heavy_turn_after_ten_minutes_is_unexplained() {
    let rows = [
        turn("a", 0, 1, (4, 100, 18_000)),
        turn("a", 10, 2, (4, 18_000, 0)),
    ];
    let causes = misses(&rows);
    assert_eq!((causes.misses, causes.ttl, causes.unexplained), (1, 0, 1));
}

#[test]
fn a_ping_inside_the_hour_refreshes_the_clock() {
    let rows = [
        turn("a", 0, 1, (4, 100, 18_000)),
        ping("a", 50, 0.1),
        turn("a", 70, 2, (4, 18_000, 0)),
    ];
    let causes = misses(&rows);
    assert_eq!((causes.misses, causes.ttl, causes.unexplained), (1, 0, 1));
}

#[test]
fn a_write_heavy_turn_with_no_previous_row_is_unexplained() {
    let rows = [turn("a", 0, 2, (4, 18_000, 0))];
    let causes = misses(&rows);
    assert_eq!((causes.misses, causes.unexplained), (1, 1));
}

#[test]
fn a_read_heavy_turn_is_not_a_miss() {
    let rows = [
        turn("a", 0, 1, (4, 100, 18_000)),
        turn("a", 90, 2, (4, 100, 18_000)),
    ];
    let causes = misses(&rows);
    assert_eq!((causes.misses, causes.turns), (0, 2));
}

#[test]
fn interleaved_runs_do_not_share_the_clock() {
    let rows = [
        turn("a", 0, 1, (4, 100, 18_000)),
        turn("b", 30, 1, (4, 100, 18_000)),
        turn("a", 61, 2, (4, 18_000, 0)),
        turn("b", 70, 2, (4, 18_000, 0)),
    ];
    let causes = misses(&rows);
    assert_eq!((causes.misses, causes.ttl, causes.unexplained), (2, 1, 1));
}

#[test]
fn period_carries_the_model_of_the_last_turn() {
    let rows = [
        turn("r", 0, 1, (1, 10, 90)),
        ping("r", 30, 0.01),
        turn("r", 40, 2, (1, 0, 100)),
    ];
    let periods = idle_periods(&entries(&rows));
    assert_eq!(periods[0].model.as_deref(), Some("claude-opus-5-5"));
}

fn runless(mut row: Value) -> Value {
    row.as_object_mut().unwrap().remove("run");
    row
}

fn runless_rows_fixture() -> Vec<Value> {
    vec![
        runless(turn("x", 0, 1, (4, 100, 0))),
        runless(ping("x", 10, 0.01)),
        runless(turn("x", 20, 2, (4, 100, 0))),
        runless(ping("x", 30, 0.01)),
    ]
}

#[test]
fn rows_without_a_run_make_no_period_and_no_miss_turns() {
    let rows = runless_rows_fixture();
    assert!(idle_periods(&entries(&rows)).is_empty());
    assert_eq!(misses(&rows).turns, 0);
    assert!(reset_writes(&entries(&rows)).is_empty());
}

#[test]
fn runless_rows_do_not_change_the_run_only_results() {
    let run_rows = vec![
        turn("a", 0, 1, (4, 100, 0)),
        ping("a", 10, 0.01),
        turn("a", 20, 2, (4, 100, 0)),
    ];
    let mut mixed = run_rows.clone();
    mixed.extend(runless_rows_fixture());
    assert_eq!(
        idle_periods(&entries(&mixed)),
        idle_periods(&entries(&run_rows))
    );
    assert_eq!(misses(&mixed), misses(&run_rows));
    assert_eq!(
        reset_writes(&entries(&mixed)),
        reset_writes(&entries(&run_rows))
    );
}
