use super::{idle_periods, outcome_counts, IdlePeriod, PeriodOutcome};
use crate::worker::TicketKeys;
use crate::worker_costs::usage_report::{parse, Entry};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

fn at(minute: u32) -> String {
    format!("2026-11-10T10:{minute:02}:00Z")
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
    assert_eq!(found[0].run.as_deref(), Some("r"));
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
    let a = found.iter().find(|p| p.run.as_deref() == Some("a"));
    let b = found.iter().find(|p| p.run.as_deref() == Some("b"));
    assert_eq!(
        a.map(|p| (p.pings, p.outcome)),
        Some((1, PeriodOutcome::Warm))
    );
    assert_eq!(
        b.map(|p| (p.pings, p.outcome)),
        Some((2, PeriodOutcome::NoNextTurn))
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
        run: None,
        pings,
        ping_cost,
        last_turn_at: None,
        last_ping_at: None,
        context_tokens: None,
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
