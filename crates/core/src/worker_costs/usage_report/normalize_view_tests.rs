use super::model_view::cost_rows;
use crate::wire::provider::{provider, ProviderId};
use crate::wire::view::Pricing;
use crate::worker_costs::window::instant;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn row(kind: &str, day: u32, process_turn: Option<u64>, cost: f64, marked: bool) -> Value {
    let mut row = json!({
        "at": format!("2026-11-{day:02}T10:00:00Z"),
        "kind": kind,
        "run": "r",
        "cost_usd": cost,
        "input_tokens": 10,
        "output_tokens": 5,
        "model": "claude-opus-5-5",
    });
    if let Some(turn) = process_turn {
        row["process_turn"] = json!(turn);
    }
    if marked {
        row["ledger_v"] = json!(2);
    }
    row
}

fn old_turn(day: u32, turn: u64, total: f64) -> Value {
    row("turn", day, Some(turn), total, false)
}

fn new_turn(day: u32, turn: u64, own: f64) -> Value {
    row("turn", day, Some(turn), own, true)
}

fn main_harness_usd(rows: &[Value], since: Option<&str>) -> f64 {
    let text: String = rows.iter().map(|row| format!("{row}\n")).collect();
    let overrides = BTreeMap::new();
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    cost_rows(&text, since.and_then(instant), &pricing)
        .iter()
        .find(|row| row.harness == "main")
        .and_then(|row| row.harness_usd.known)
        .unwrap()
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

#[test]
fn old_rows_report_the_last_total_of_each_process() {
    let rows = [
        old_turn(1, 1, 0.5),
        old_turn(1, 2, 1.0),
        old_turn(2, 1, 0.3),
    ];
    assert!(close(main_harness_usd(&rows, None), 1.3));
}

#[test]
fn new_rows_report_the_sum_of_their_own_costs() {
    let rows = [
        new_turn(1, 1, 0.5),
        row("ping", 1, None, 0.05, true),
        new_turn(1, 2, 0.25),
    ];
    assert!(close(main_harness_usd(&rows, None), 0.8));
}

#[test]
fn old_rows_then_new_rows_in_one_process_add_up() {
    let rows = [
        old_turn(1, 1, 0.5),
        old_turn(1, 2, 1.0),
        new_turn(2, 3, 0.1),
        new_turn(2, 4, 0.2),
    ];
    assert!(close(main_harness_usd(&rows, None), 1.3));
}

#[test]
fn a_window_subtracts_what_ran_before_it() {
    let rows = [
        old_turn(1, 1, 0.5),
        old_turn(1, 2, 1.0),
        old_turn(2, 3, 1.4),
        new_turn(2, 4, 0.1),
    ];
    assert!(close(
        main_harness_usd(&rows, Some("2026-11-02T00:00:00Z")),
        0.5
    ));
}

#[test]
fn a_decreasing_total_inside_the_window_reports_the_last_total() {
    let rows = [old_turn(2, 1, 1.0), old_turn(2, 2, 0.75)];
    assert!(close(
        main_harness_usd(&rows, Some("2026-11-02T00:00:00Z")),
        0.75
    ));
}
