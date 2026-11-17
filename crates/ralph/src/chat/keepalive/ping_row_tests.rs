use super::*;
use crate::stream_processor::StreamProcessorResult;
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
        cost_before: None,
        logged_before: before,
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
