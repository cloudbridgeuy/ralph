use super::*;
use crate::wire::provider::{provider, ProviderId};
use crate::wire::view::Pricing;
use crate::worker_costs::usage_report::model_view::costs_view;
use std::collections::BTreeMap;

const FIXTURE: &str = include_str!("fixtures/old_rows.jsonl");
const RUN_A: &str = "100-2026-11-01T09:00:00+00:00";
const RUN_B: &str = "200-2026-11-01T10:00:00+00:00";

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

fn assert_cost(cost: Option<SessionCost>, total: f64, delta: Option<f64>, missing: bool) {
    let cost = cost.unwrap();
    assert!(close(cost.total, total), "total {}", cost.total);
    match (cost.delta, delta) {
        (Some(actual), Some(expected)) => assert!(close(actual, expected), "delta {actual}"),
        (actual, expected) => assert_eq!(actual, expected),
    }
    assert_eq!(cost.missing, missing);
}

fn view(since: Option<&str>) -> String {
    let overrides = BTreeMap::new();
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    costs_view(FIXTURE, since.and_then(instant), &pricing)
}

#[test]
fn fixture_has_no_marker_and_enough_rows() {
    assert_eq!(FIXTURE.lines().count(), 41);
    assert!(!FIXTURE.contains("ledger_v"));
}

#[test]
fn run_a_session_cost_is_the_sum_of_process_last_totals() {
    let rows = session_rows(FIXTURE, RUN_A);
    assert_cost(session_cost(&rows), 3.1, Some(0.08), false);
}

#[test]
fn run_b_session_cost_is_the_sum_of_process_last_totals() {
    let rows = session_rows(FIXTURE, RUN_B);
    assert_cost(session_cost(&rows), 1.26, Some(0.09), false);
}

#[test]
fn a_run_ending_on_a_null_cost_is_missing_with_no_delta() {
    let rows = session_rows(FIXTURE, RUN_B);
    let cut = rows.iter().rposition(|row| row.cost_usd.is_none()).unwrap();
    assert_cost(session_cost(&rows[..=cut]), 0.85, None, true);
}

#[test]
fn process_usage_reads_the_last_turn_and_the_session_cost() {
    let usage = process_usage(&session_rows(FIXTURE, RUN_A)).unwrap();
    assert_eq!(usage.process_turn, 6);
    assert_eq!(usage.context_tokens, Some(70_000));
    assert!(!usage.write_heavy);
    assert_eq!(usage.request_at, instant("2026-11-02T16:00:00+00:00"));
    assert_eq!(usage.reset, None);
    assert_cost(usage.cost, 3.1, Some(0.08), false);
}

#[test]
fn costs_view_of_the_whole_fixture() {
    let expected = [
        "| HARNESS | MODEL | RUNS | INPUT | OUTPUT | CACHE READ | CACHE WRITE | 5M | 1H | REASONING | USD | HARNESS USD |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
        "| main | claude-opus-5-5 | 2 | 184 | 20.7k | 6.4M | 270.4k | – | – | – | ~3.87 | 4.36 |",
        "| nap | claude-haiku-5-5 | 5 | 10 | 1.1k | 11.2k | 19.7k | – | – | – | ~0.00 | 0.01 |",
        "| ask | claude-sonnet-5-5 | 3 | 33 | 1.3k | 0 | 10.5k | – | – | – | ~0.06 | 0.05 |",
        "| total |  |  | 227 | 23.2k | 6.4M | 300.6k | – | – | – | ~3.93 | 4.41 |",
    ]
    .join("\n");
    assert_eq!(view(None), expected);
}

#[test]
fn costs_view_since_the_second_day_subtracts_the_baseline() {
    let expected = [
        "| HARNESS | MODEL | RUNS | INPUT | OUTPUT | CACHE READ | CACHE WRITE | 5M | 1H | REASONING | USD | HARNESS USD |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
        "| main | claude-opus-5-5 | 2 | 94 | 10.3k | 2.9M | 99.6k | – | – | – | ~1.60 | 1.91 |",
        "| nap | claude-haiku-5-5 | 2 | 4 | 560 | 5.5k | 9.3k | – | – | – | ~0.00 | 0.00 |",
        "| ask | claude-sonnet-5-5 | 2 | 23 | 950 | 0 | 7.5k | – | – | – | ~0.04 | 0.04 |",
        "| total |  |  | 121 | 11.8k | 2.9M | 116.4k | – | – | – | ~1.64 | 1.95 |",
    ]
    .join("\n");
    assert_eq!(view(Some("2026-11-02T00:00:00+00:00")), expected);
}
