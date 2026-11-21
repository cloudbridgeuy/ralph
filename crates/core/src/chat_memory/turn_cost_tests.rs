use super::*;
use crate::chat_backend::CLAUDE;
use serde_json::Value;

fn row(kind: CallSite, total: f64) -> LedgerRow {
    usage_record(
        kind,
        "t".into(),
        &IterationCosts {
            cost_usd: Some(total),
            duration_ms: None,
            usage: Some(crate::stream::Usage::default()),
            context_tokens: None,
            terminal_reason: None,
            model_usage: Default::default(),
        },
        &IterationMetadata::new(),
        &CLAUDE,
    )
    .unwrap()
}

fn json(row: LedgerRow) -> Value {
    serde_json::to_value(row).unwrap()
}

#[test]
fn an_unmarked_row_keeps_its_cost_and_has_no_ledger_version() {
    let value = json(row(CallSite::Ping, 2.0));
    assert_eq!(value["cost_usd"], 2.0);
    assert!(value.get("ledger_v").is_none());
}

#[test]
fn a_turn_row_stores_its_own_cost_and_ledger_version_two() {
    let value = json(row(CallSite::Ping, 2.0).with_turn_cost(Some(1.5)));
    assert_eq!(value["cost_usd"], 0.5);
    assert_eq!(value["ledger_v"], 2);
}

#[test]
fn the_first_turn_of_a_process_keeps_its_whole_total() {
    let value = json(row(CallSite::Ping, 0.58).with_turn_cost(Some(0.0)));
    assert_eq!(value["cost_usd"], 0.58);
    assert_eq!(value["ledger_v"], 2);
}

#[test]
fn an_unknown_running_total_stores_no_cost_but_the_marker() {
    let value = json(row(CallSite::Ping, 2.0).with_turn_cost(None));
    assert!(value["cost_usd"].is_null());
    assert_eq!(value["ledger_v"], 2);
}

#[test]
fn a_row_without_a_total_stays_without_a_cost() {
    let mut bare = row(CallSite::Ping, 2.0);
    bare.cost_usd = None;
    let value = json(bare.with_turn_cost(Some(1.0)));
    assert!(value["cost_usd"].is_null());
    assert_eq!(value["ledger_v"], 2);
}
