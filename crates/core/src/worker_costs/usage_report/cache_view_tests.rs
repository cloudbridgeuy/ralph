use super::{process_costs, summarize, ProcessCost, ProcessStart};
use crate::chat_process::ResetReason;
use crate::worker::TicketKeys;
use crate::worker_costs::usage_report::{parse, Entry};
use serde_json::{json, Value};

fn row(kind: &str, run: &str, process_turn: Option<u64>, cost: f64, marked: bool) -> Value {
    let mut row = json!({
        "at": "2026-11-10T10:00:00Z",
        "kind": kind,
        "run": run,
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

fn entries(rows: &[Value]) -> Vec<Entry> {
    let text: String = rows.iter().map(|row| format!("{row}\n")).collect();
    parse(&text, &[], &TicketKeys::default()).entries
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

fn cost(cost: f64, turns: u64, start: ProcessStart) -> ProcessCost {
    ProcessCost {
        run: None,
        cost,
        turns,
        pings: 0,
        start,
        complete: start != ProcessStart::Cut,
    }
}

#[test]
fn old_rows_sum_to_the_final_running_total() {
    let rows = [
        row("turn", "r", Some(1), 0.28, false),
        row("ping", "r", None, 0.30, false),
        row("turn", "r", Some(2), 0.55, false),
    ];
    let found = process_costs(&entries(&rows));
    assert_eq!(found.len(), 1);
    assert!(close(found[0].cost, 0.55));
    assert_eq!((found[0].turns, found[0].pings), (2, 1));
    assert_eq!(found[0].start, ProcessStart::NewRun);
    assert!(found[0].complete);
    assert_eq!(found[0].run.as_deref(), Some("r"));
}

#[test]
fn new_rows_sum_their_own_costs() {
    let rows = [
        row("turn", "r", Some(1), 0.28, true),
        row("ping", "r", None, 0.02, true),
        row("turn", "r", Some(2), 0.25, true),
    ];
    let found = process_costs(&entries(&rows));
    assert_eq!(found.len(), 1);
    assert!(close(found[0].cost, 0.55));
    assert_eq!((found[0].turns, found[0].pings), (2, 1));
}

#[test]
fn old_and_new_rows_mix_in_one_run() {
    let rows = [
        row("turn", "r", Some(1), 0.28, false),
        row("turn", "r", Some(2), 0.55, false),
        row("turn", "r", Some(1), 0.10, true),
        row("turn", "r", Some(2), 0.20, true),
    ];
    let found = process_costs(&entries(&rows));
    assert_eq!(found.len(), 2);
    assert!(close(found[0].cost, 0.55));
    assert!(close(found[1].cost, 0.30));
}

#[test]
fn interleaved_runs_stay_separate() {
    let rows = [
        row("turn", "a", Some(1), 0.10, true),
        row("turn", "b", Some(1), 0.20, true),
        row("turn", "a", Some(2), 0.30, true),
        row("turn", "b", Some(2), 0.40, true),
    ];
    let found = process_costs(&entries(&rows));
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].run.as_deref(), Some("a"));
    assert!(close(found[0].cost, 0.40));
    assert_eq!(found[1].run.as_deref(), Some("b"));
    assert!(close(found[1].cost, 0.60));
}

#[test]
fn a_cut_leading_process_counts_cost_but_is_incomplete() {
    let rows = [
        row("turn", "r", Some(2), 0.30, true),
        row("turn", "r", Some(3), 0.20, true),
        row("turn", "r", Some(1), 0.10, true),
    ];
    let found = process_costs(&entries(&rows));
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].start, ProcessStart::Cut);
    assert!(!found[0].complete);
    assert!(close(found[0].cost, 0.50));
    assert!(found[1].complete);
    let summary = summarize(&found);
    assert_eq!(summary.processes, 2);
    assert!(close(summary.total, 0.60));
    assert_eq!(summary.median_turns, Some(1));
    assert_eq!(summary.by_start, vec![("new run".to_string(), 1, 0.10)]);
}

#[test]
fn reset_reasons_mark_the_process_start() {
    let mut tokens = row("turn", "r", Some(1), 0.1, true);
    tokens["reset"] = json!("tokens");
    let mut turns = row("turn", "r", Some(1), 0.1, true);
    turns["reset"] = json!("turns");
    let mut restart = row("turn", "r", Some(1), 0.1, true);
    restart["reset"] = json!("restart");
    let fresh = row("turn", "r", Some(1), 0.1, true);
    let found = process_costs(&entries(&[tokens, turns, restart, fresh]));
    let starts: Vec<ProcessStart> = found.iter().map(|found| found.start).collect();
    assert_eq!(
        starts,
        vec![
            ProcessStart::Reset(ResetReason::Tokens),
            ProcessStart::Reset(ResetReason::Turns),
            ProcessStart::Reset(ResetReason::Restart),
            ProcessStart::NewRun,
        ]
    );
}

#[test]
fn rows_without_a_process_turn_are_cut() {
    let found = process_costs(&entries(&[row("ping", "r", None, 0.05, true)]));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].start, ProcessStart::Cut);
    assert_eq!((found[0].turns, found[0].pings), (0, 1));
}

#[test]
fn non_main_rows_are_ignored() {
    let rows = [
        row("turn", "r", Some(1), 0.10, true),
        row("nap", "r", None, 9.0, true),
    ];
    let found = process_costs(&entries(&rows));
    assert_eq!(found.len(), 1);
    assert!(close(found[0].cost, 0.10));
}

#[test]
fn summarize_an_empty_slice() {
    let summary = summarize(&[]);
    assert_eq!(summary.processes, 0);
    assert_eq!(summary.total, 0.0);
    assert_eq!(summary.mean, None);
    assert_eq!(summary.median, None);
    assert_eq!(summary.median_turns, None);
    assert!(summary.by_start.is_empty());
}

#[test]
fn summarize_odd_count_takes_the_middle() {
    let costs = [
        cost(3.0, 3, ProcessStart::NewRun),
        cost(1.0, 1, ProcessStart::NewRun),
        cost(2.0, 5, ProcessStart::NewRun),
    ];
    let summary = summarize(&costs);
    assert!(close(summary.total, 6.0));
    assert!(close(summary.mean.unwrap(), 2.0));
    assert!(close(summary.median.unwrap(), 2.0));
    assert_eq!(summary.median_turns, Some(3));
}

#[test]
fn summarize_even_count_averages_the_middle_pair() {
    let costs = [
        cost(1.0, 2, ProcessStart::NewRun),
        cost(4.0, 4, ProcessStart::NewRun),
        cost(2.0, 6, ProcessStart::NewRun),
        cost(10.0, 8, ProcessStart::NewRun),
    ];
    let summary = summarize(&costs);
    assert!(close(summary.median.unwrap(), 3.0));
    assert_eq!(summary.median_turns, Some(4));
}

#[test]
fn summarize_groups_by_start_in_fixed_order() {
    let costs = [
        cost(1.0, 1, ProcessStart::NewRun),
        cost(2.0, 1, ProcessStart::Reset(ResetReason::Restart)),
        cost(3.0, 1, ProcessStart::Reset(ResetReason::Tokens)),
        cost(4.0, 1, ProcessStart::Reset(ResetReason::Tokens)),
        cost(5.0, 1, ProcessStart::Cut),
    ];
    let summary = summarize(&costs);
    assert_eq!(
        summary.by_start,
        vec![
            ("tokens".to_string(), 2, 7.0),
            ("restart".to_string(), 1, 2.0),
            ("new run".to_string(), 1, 1.0),
        ]
    );
    assert!(close(summary.total, 15.0));
}
