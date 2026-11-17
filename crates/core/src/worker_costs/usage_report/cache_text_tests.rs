use super::{miss_lines, period_lines, process_lines, reset_lines, skipped_lines};
use crate::chat_process::ResetReason;
use crate::wire::provider::{provider, ProviderId};
use crate::wire::view::Pricing;
use crate::worker::TicketKeys;
use crate::worker_costs::usage_report::cache_periods::{MissCauses, OutcomeCounts, ResetWrites};
use crate::worker_costs::usage_report::cache_saving::SAVING_RULE;
use crate::worker_costs::usage_report::cache_view::{ProcessStart, ProcessSummary};
use crate::worker_costs::usage_report::text::{choose, render, Inputs, View};
use crate::worker_costs::usage_report::Filter;
use crate::worker_costs::NO_USAGE;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn at(minute: u32) -> String {
    format!("2026-11-10T{:02}:{:02}:00Z", 10 + minute / 60, minute % 60)
}

fn turn(minute: u32, process_turn: u64, tokens: (u64, u64, u64)) -> Value {
    let (input, write, read) = tokens;
    let mut row = json!({
        "at": at(minute),
        "kind": "turn",
        "run": "r",
        "cost_usd": 0.5,
        "context_tokens": input + write + read,
        "input_tokens": input,
        "cache_creation_input_tokens": write,
        "cache_read_input_tokens": read,
        "output_tokens": 5,
        "process_turn": process_turn,
        "model": "claude-opus-5-5",
        "ledger_v": 2,
    });
    if process_turn == 1 {
        row["reset"] = json!("tokens");
    }
    row
}

fn ping(minute: u32) -> Value {
    json!({
        "at": at(minute),
        "kind": "ping",
        "run": "r",
        "cost_usd": 0.01,
        "input_tokens": 3,
        "output_tokens": 1,
        "model": "claude-opus-5-5",
        "ledger_v": 2,
    })
}

fn ask() -> Value {
    json!({
        "at": at(5),
        "kind": "ask",
        "cost_usd": 0.2,
        "input_tokens": 3,
        "output_tokens": 1,
        "model": "claude-opus-5-5",
        "ledger_v": 2,
    })
}

fn fixture() -> Vec<Value> {
    vec![
        turn(0, 1, (10, 100_000, 0)),
        turn(1, 2, (10, 10, 100_000)),
        ping(30),
        ping(55),
        turn(70, 3, (10, 10, 100_000)),
        ping(100),
        turn(200, 4, (10, 100_000, 0)),
        ping(210),
    ]
}

fn text_of(rows: &[Value]) -> String {
    rows.iter().map(|row| format!("{row}\n")).collect()
}

fn shown(usage: &str, filter: &Filter) -> String {
    let provider = provider(ProviderId::Anthropic);
    let overrides = BTreeMap::new();
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    let tickets = TicketKeys::default();
    let inputs = Inputs {
        usage,
        workers: "",
        tickets: &tickets,
    };
    render(View::Cache, &inputs, filter, &Utc, &pricing)
}

#[test]
fn render_keeps_the_model_table_first_and_adds_the_sections() {
    let out = shown(&text_of(&fixture()), &Filter::default());
    let table = out.split("\n\n").next().unwrap();
    assert!(
        table.starts_with("usage cache by model\nclaude-opus-5-5"),
        "{out}"
    );
    assert!(!table.contains("cost per claude process"), "{out}");
    for heading in [
        "cost per claude process",
        "keepalive per idle period",
        "first turn after a reset",
        "cache misses",
    ] {
        assert!(
            out.contains(&format!("\n\n{heading}\n")),
            "{heading}\n{out}"
        );
    }
    assert!(out.contains(SAVING_RULE), "{out}");
    assert!(out.contains("est."), "{out}");
    assert!(
        out.contains("4 turns") || out.contains("misses in 4 turns"),
        "{out}"
    );
}

#[test]
fn render_puts_the_malformed_footer_last() {
    let usage = format!("{}not json\n", text_of(&fixture()));
    let out = shown(&usage, &Filter::default());
    assert!(out.ends_with("malformed lines skipped: 1"), "{out}");
    assert!(out.contains("cache misses"), "{out}");
}

#[test]
fn render_of_an_empty_ledger_prints_only_the_table() {
    let out = shown("", &Filter::default());
    assert_eq!(out, format!("usage cache by model\n{NO_USAGE}"));
}

#[test]
fn render_without_main_rows_prints_no_sections() {
    let out = shown(&text_of(&[ask()]), &Filter::default());
    assert!(!out.contains("cost per claude process"), "{out}");
    assert!(!out.contains("cache misses"), "{out}");
    assert!(out.starts_with("usage cache by model"), "{out}");
}

#[test]
fn render_with_a_kind_filter_that_removes_main_rows_prints_no_sections() {
    let mut rows = fixture();
    rows.push(ask());
    let filter = Filter {
        kind: Some("ask".to_string()),
        ..Filter::default()
    };
    let out = shown(&text_of(&rows), &filter);
    assert!(!out.contains("cost per claude process"), "{out}");
    assert!(!out.contains("saving estimate"), "{out}");
}

#[test]
fn the_cache_view_still_has_no_chart() {
    assert_eq!(
        choose(View::Cache, false, Some("x.png".into()), || true),
        Err("the cache view has no chart")
    );
}

fn summary(by_start: Vec<(String, usize, f64)>) -> ProcessSummary {
    ProcessSummary {
        processes: 3,
        total: 3.0,
        mean: Some(1.0),
        median: Some(0.5),
        median_turns: Some(4),
        by_start,
    }
}

#[test]
fn process_lines_list_totals_and_starts() {
    let lines = process_lines(&summary(vec![("tokens".to_string(), 3, 3.0)]), 0);
    assert_eq!(
        lines[0],
        "3 processes  total $3.00  mean $1.00  median $0.50  median turns 4"
    );
    assert_eq!(lines[1], "  tokens  3 processes  $3.00");
    assert_eq!(lines.len(), 2);
}

#[test]
fn process_lines_note_cut_processes_once() {
    let lines = process_lines(&summary(vec![]), 2);
    assert_eq!(
        lines.last().unwrap(),
        "2 process(es) cut by the window: cost counted, no turn stats"
    );
}

#[test]
fn process_lines_use_dashes_without_medians() {
    let mut empty = summary(vec![]);
    empty.mean = None;
    empty.median = None;
    empty.median_turns = None;
    let lines = process_lines(&empty, 0);
    assert_eq!(
        lines[0],
        "3 processes  total $3.00  mean -  median -  median turns -"
    );
}

#[test]
fn period_lines_show_counts_and_outcomes() {
    let counts = OutcomeCounts {
        periods: 3,
        pings: 7,
        ping_cost: 0.07,
        warm: 1,
        cold: 1,
        no_next: 1,
    };
    assert_eq!(
        period_lines(&counts),
        vec![
            "3 periods  7 pings  $0.07 ping cost".to_string(),
            "next turn warm 1  cold 1  no next turn 1".to_string()
        ]
    );
}

#[test]
fn reset_lines_label_groups_and_missing_ratios() {
    let lines = reset_lines(&[
        ResetWrites {
            start: ProcessStart::Reset(ResetReason::Tokens),
            turns: 2,
            mean_ratio: Some(0.5),
            mean_write_tokens: Some(1200.0),
        },
        ResetWrites {
            start: ProcessStart::NewRun,
            turns: 1,
            mean_ratio: None,
            mean_write_tokens: Some(0.0),
        },
    ]);
    assert_eq!(
        lines,
        vec![
            "tokens  2 turns  writes 50% of context  (mean 1.2k tokens)".to_string(),
            "new run  1 turns  no ratio  (mean 0 tokens)".to_string()
        ]
    );
}

#[test]
fn miss_lines_name_every_cause() {
    let causes = MissCauses {
        misses: 4,
        reset: 1,
        ttl: 2,
        unexplained: 0,
        new_run: 1,
        turns: 9,
    };
    assert_eq!(
        miss_lines(&causes),
        vec!["4 misses in 9 turns: reset 1  ttl expiry 2  unexplained 0  new run 1".to_string()]
    );
}

fn runless_ping(minute: u32) -> Value {
    let mut row = ping(minute);
    row.as_object_mut().unwrap().remove("run");
    row
}

#[test]
fn render_says_how_many_main_rows_without_a_run_it_skipped() {
    let mut rows = fixture();
    rows.extend([runless_ping(300), runless_ping(301), runless_ping(302)]);
    let out = shown(&text_of(&rows), &Filter::default());
    assert!(
        out.contains("3 main rows without a run skipped (no chat to join them to)"),
        "{out}"
    );
    assert!(out.contains("cache misses"), "{out}");
    assert!(out.contains(SAVING_RULE), "{out}");
}

#[test]
fn render_uses_the_singular_for_one_runless_row() {
    let mut rows = fixture();
    rows.push(runless_ping(300));
    let out = shown(&text_of(&rows), &Filter::default());
    assert!(
        out.contains("1 main row without a run skipped (no chat to join them to)"),
        "{out}"
    );
}

#[test]
fn render_without_runless_rows_has_no_skipped_line() {
    let out = shown(&text_of(&fixture()), &Filter::default());
    assert!(!out.contains("without a run skipped"), "{out}");
}

#[test]
fn skipped_lines_pluralize_and_vanish_at_zero() {
    assert!(skipped_lines(0).is_empty());
    assert_eq!(
        skipped_lines(1),
        vec!["1 main row without a run skipped (no chat to join them to)".to_string()]
    );
    assert_eq!(
        skipped_lines(2),
        vec!["2 main rows without a run skipped (no chat to join them to)".to_string()]
    );
}
