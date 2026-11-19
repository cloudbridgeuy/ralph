use super::*;
use crate::subprocess::SubprocessError;
use ralph_core::stream::blocks::StreamProcessorResult;
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

fn timed_out(
    partial: StreamingSubprocessResult,
) -> Result<StreamingSubprocessResult, SubprocessError> {
    Err(SubprocessError::Timeout {
        timeout_secs: 1,
        partial_result: Box::new(partial),
    })
}

#[test]
fn a_timed_out_ping_with_usage_writes_one_row_for_its_own_spend() {
    let outcome = timed_out(result(Some(1.5), Some(1.0)));
    let rows: Vec<Value> = recorded(&outcome)
        .and_then(|partial| ping_row("t".into(), partial, &IterationMetadata::new()))
        .into_iter()
        .map(|row| serde_json::to_value(row).unwrap())
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["cost_usd"], 0.5);
    assert_eq!(rows[0]["ledger_v"], 2);
    assert_eq!(rows[0]["kind"], "ping");
}

#[test]
fn a_timed_out_ping_without_usage_has_no_row() {
    let mut bare = result(Some(1.5), Some(1.0));
    bare.stream_result.costs.usage = None;
    let outcome = timed_out(bare);
    let row = recorded(&outcome)
        .and_then(|partial| ping_row("t".into(), partial, &IterationMetadata::new()));
    assert!(row.is_none());
}

#[test]
fn a_ping_that_ended_in_any_other_error_has_no_record() {
    assert!(recorded(&Err(SubprocessError::Signaled)).is_none());
}

#[test]
fn an_interrupted_partial_is_returned_by_the_shared_recorder() {
    let outcome = Err(SubprocessError::Interrupted {
        partial_result: Box::new(result(Some(1.0), Some(0.0))),
    });
    assert!(recorded(&outcome).is_some());
}

#[test]
fn a_stale_ping_has_no_record() {
    assert!(recorded(&Err(SubprocessError::Stale)).is_none());
}

#[test]
fn a_finished_ping_is_recorded() {
    assert!(recorded(&Ok(result(Some(1.0), Some(0.0)))).is_some());
}
