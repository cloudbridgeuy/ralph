use super::*;

#[test]
fn the_first_turn_of_a_process_pays_its_whole_total() {
    assert_eq!(turn_cost(0.58, Some(0.0)), Some(0.58));
}

#[test]
fn a_later_turn_pays_the_growth_of_the_running_total() {
    assert_eq!(turn_cost(2.0, Some(1.5)), Some(0.5));
}

#[test]
fn a_ping_pays_the_growth_of_the_running_total() {
    assert_eq!(turn_cost(1.55, Some(1.5)), Some(1.55 - 1.5));
}

#[test]
fn a_restart_starts_from_zero() {
    assert_eq!(turn_cost(0.4, Some(0.0)), Some(0.4));
}

#[test]
fn an_unknown_running_total_gives_no_cost() {
    assert_eq!(turn_cost(2.0, None), None);
}

#[test]
fn a_total_that_went_down_gives_a_negative_cost() {
    assert_eq!(turn_cost(1.0, Some(1.5)), Some(-0.5));
}

#[test]
fn an_old_row_has_no_ledger_version() {
    let row: UsageLine = serde_json::from_str(r#"{"kind":"turn","cost_usd":0.5}"#).unwrap();
    assert_eq!(row.ledger_v, None);
}

#[test]
fn a_new_row_reads_its_ledger_version() {
    let row: UsageLine =
        serde_json::from_str(r#"{"kind":"turn","cost_usd":0.5,"ledger_v":2}"#).unwrap();
    assert_eq!(row.ledger_v, Some(2));
}
