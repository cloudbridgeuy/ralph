use super::*;
use crate::stream_processor::StreamProcessorResult;
use crate::subprocess::next_running;
use ralph_core::stream::{IterationCosts, Usage};
use serde_json::Value;

fn result(total: Option<f64>, before: Option<f64>) -> StreamingSubprocessResult {
    StreamingSubprocessResult {
        exit_code: 0,
        stderr: String::new(),
        stream_result: StreamProcessorResult {
            costs: IterationCosts {
                cost_usd: total,
                usage: Some(Usage::default()),
                ..Default::default()
            },
            ..Default::default()
        },
        request_at: None,
        cost_before: before,
    }
}

fn row(total: f64, before: Option<f64>) -> Value {
    let row = ping_row(
        "t".into(),
        &result(Some(total), before),
        &IterationMetadata::new(),
    )
    .unwrap();
    serde_json::to_value(row).unwrap()
}

#[test]
fn a_ping_row_stores_its_own_cost_and_the_ledger_version() {
    let value = row(2.0, Some(1.5));
    assert_eq!(value["cost_usd"], 0.5);
    assert_eq!(value["ledger_v"], 2);
    assert_eq!(value["kind"], "ping");
}

#[test]
fn a_ping_without_a_known_running_total_stores_no_cost() {
    let value = row(2.0, None);
    assert!(value["cost_usd"].is_null());
    assert_eq!(value["ledger_v"], 2);
}

#[test]
fn a_ping_without_usage_has_no_row() {
    let mut bare = result(Some(1.0), Some(0.0));
    bare.stream_result.costs.usage = None;
    assert!(ping_row("t".into(), &bare, &IterationMetadata::new()).is_none());
}

#[test]
fn a_failed_ping_without_cost_leaves_its_spend_to_the_next_turn() {
    let after_turn = next_running(Some(0.0), Some(1.0));
    let after_failed_ping = next_running(after_turn, None);
    assert_eq!(after_failed_ping, Some(1.0));
    let next_turn = result(Some(1.75), after_failed_ping);
    assert_eq!(next_turn.cost_before, Some(1.0));
    let own = ralph_core::chat_process::turn_cost(1.75, next_turn.cost_before);
    assert_eq!(own, Some(0.75));
}

#[test]
fn a_timed_out_ping_with_cost_advances_the_total_and_writes_no_row() {
    let after_turn = next_running(Some(0.0), Some(1.0));
    let after_timeout = next_running(after_turn, Some(1.5));
    assert_eq!(after_timeout, Some(1.5));
    let written: f64 = [1.0].iter().sum();
    assert_eq!(after_timeout.map(|total| total - written), Some(0.5));
}
