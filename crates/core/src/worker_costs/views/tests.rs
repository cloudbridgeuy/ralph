use super::*;
use crate::wire::price::PriceOverride;
use crate::wire::provider::{provider, ProviderId};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn view_of(view: View, usage: &[Value], workers: &[Value]) -> String {
    let overrides: BTreeMap<String, PriceOverride> = BTreeMap::new();
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    let inputs = Inputs {
        usage: &jsonl(usage),
        workers: &jsonl(workers),
    };
    costs_by_view(view, &inputs, &pricing)
}

fn jsonl(rows: &[Value]) -> String {
    rows.iter().map(|row| format!("{row}\n")).collect()
}

fn worker(id: &str, at: &str, model: &str) -> Value {
    json!({
        "kind": "worker", "at": at, "worker_id": id, "turn_id": 1, "harness": "claude",
        "model": model, "session_id": "s", "cost_usd": null,
        "input_tokens": 3, "output_tokens": 7, "cache_creation_input_tokens": 50,
        "cache_read_input_tokens": 400, "cache_write_5m": 20, "cache_write_1h": 30,
    })
}

fn nap(model: &str) -> Value {
    json!({
        "kind": "nap", "at": "2026-11-01T10:00:00Z", "harness": "claude", "model": model,
        "cost_usd": 0.01, "input_tokens": 1, "output_tokens": 2,
        "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0,
    })
}

fn turn() -> Value {
    json!({
        "kind": "turn", "at": "2026-11-01T10:00:00Z", "trigger": "user", "run": "r1",
        "process_turn": 1, "cost_usd": 0.5, "input_tokens": 10, "output_tokens": 5,
        "cache_creation_input_tokens": 100, "cache_read_input_tokens": 1000,
        "harness": "claude", "model": "claude-opus-5-5",
    })
}

fn with_kind(kind: &str) -> Value {
    let mut row = nap("claude-haiku-5-5");
    row["kind"] = json!(kind);
    row
}

fn spawn(id: &str, at: &str, ticket: Option<&str>, harness: &str) -> Value {
    json!({
        "v": 1, "at": at, "store": "/s", "worker_id": id, "event": "spawn",
        "ticket": ticket, "harness": harness, "model": null,
    })
}

fn close(id: &str, at: &str) -> Value {
    json!({
        "v": 1, "at": at, "store": "/s", "worker_id": id, "event": "close",
        "outcome": "merged",
    })
}

fn labels(view: &str) -> Vec<String> {
    view.lines()
        .skip(2)
        .map(|line| {
            line.split(" | ")
                .next()
                .unwrap()
                .trim_start_matches("| ")
                .to_string()
        })
        .collect()
}

fn row<'a>(view: &'a str, label: &str) -> Vec<&'a str> {
    view.lines()
        .find(|line| line.starts_with(&format!("| {label} | ")))
        .unwrap()
        .trim_matches(|c| c == '|' || c == ' ')
        .split(" | ")
        .collect()
}

#[test]
fn role_view_groups_by_role_and_keeps_merge_apart_from_nap() {
    let view = view_of(
        View::Role,
        &[
            worker("w1", "2026-11-01T10:00:00Z", "claude-opus-5-5"),
            with_kind("subagent"),
            with_kind("ask"),
            with_kind("merge"),
            nap("claude-haiku-5-5"),
            turn(),
        ],
        &[],
    );
    assert!(view.starts_with(
        "| ROLE | INPUT | OUTPUT | CACHE READ | CACHE WRITE | 5M | 1H | REASONING | USD |"
    ));
    assert_eq!(
        labels(&view),
        ["main", "nap", "merge", "ask", "subagent", "worker", "total"]
    );
}

#[test]
fn role_view_sums_tokens_and_prices_the_rows() {
    let view = view_of(
        View::Role,
        &[
            worker("w1", "2026-11-01T10:00:00Z", "claude-opus-5-5"),
            worker("w2", "2026-11-01T10:00:00Z", "claude-sonnet-5-5"),
        ],
        &[],
    );
    let worker = row(&view, "worker");
    assert_eq!(worker[1..8], ["6", "14", "800", "100", "40", "60", "–"]);
    assert!(worker[8].starts_with('~'));
    assert_eq!(row(&view, "total")[1..8], worker[1..8]);
}

#[test]
fn views_with_no_valid_rows_say_no_usage() {
    let other = json!({"kind": "other", "at": "2026-11-01T10:00:00Z"});
    for view in [View::Role, View::Ticket, View::Harness] {
        assert_eq!(view_of(view, std::slice::from_ref(&other), &[]), NO_USAGE);
    }
}

#[test]
fn harness_view_merges_the_models_of_a_harness() {
    let view = view_of(
        View::Harness,
        &[
            worker("w1", "2026-11-01T10:00:00Z", "claude-opus-5-5"),
            worker("w2", "2026-11-01T10:00:00Z", "claude-sonnet-5-5"),
            nap("claude-haiku-5-5"),
        ],
        &[],
    );
    assert!(view.starts_with("| HARNESS | INPUT |"));
    assert_eq!(labels(&view), ["nap", "claude", "total"]);
    assert_eq!(row(&view, "claude")[1], "6");
}

#[test]
fn model_view_is_the_harness_and_model_table() {
    let usage = [worker("w1", "2026-11-01T10:00:00Z", "claude-opus-5-5")];
    let view = view_of(View::HarnessModel, &usage, &[]);
    assert!(view.starts_with("| HARNESS | MODEL | RUNS |"));
}

#[test]
fn ticket_view_joins_worker_rows_to_their_ticket() {
    let view = view_of(
        View::Ticket,
        &[
            worker("w1", "2026-11-01T10:30:00Z", "claude-opus-5-5"),
            worker("w1", "2026-11-01T10:40:00Z", "claude-opus-5-5"),
            worker("w2", "2026-11-01T10:30:00Z", "claude-opus-5-5"),
        ],
        &[
            spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1"), "claude"),
            spawn("w2", "2026-11-01T10:00:00Z", Some("GUZ-2"), "claude"),
            close("w1", "2026-11-01T11:00:00Z"),
        ],
    );
    assert!(view.starts_with("| TICKET | INPUT |"));
    assert_eq!(labels(&view), ["GUZ-1", "GUZ-2", "total"]);
    assert_eq!(row(&view, "GUZ-1")[1], "6");
    assert_eq!(row(&view, "GUZ-2")[1], "3");
}

#[test]
fn a_reused_worker_id_lands_on_each_ticket_by_time() {
    let view = view_of(
        View::Ticket,
        &[
            worker("w1", "2026-11-01T10:30:00Z", "claude-opus-5-5"),
            worker("w1", "2026-11-02T10:30:00Z", "claude-opus-5-5"),
            worker("w1", "2026-11-02T10:31:00Z", "claude-opus-5-5"),
        ],
        &[
            spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1"), "claude"),
            close("w1", "2026-11-01T11:00:00Z"),
            spawn("w1", "2026-11-02T10:00:00Z", Some("GUZ-2"), "claude"),
        ],
    );
    assert_eq!(row(&view, "GUZ-1")[1], "3");
    assert_eq!(row(&view, "GUZ-2")[1], "6");
}

#[test]
fn rows_without_a_ticket_share_one_unknown_row_sorted_last() {
    let view = view_of(
        View::Ticket,
        &[
            turn(),
            nap("claude-haiku-5-5"),
            worker("w9", "2026-11-01T10:30:00Z", "claude-opus-5-5"),
            worker("w1", "2026-11-03T10:30:00Z", "claude-opus-5-5"),
            worker("w2", "2026-11-01T10:30:00Z", "claude-opus-5-5"),
        ],
        &[
            spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1"), "claude"),
            close("w1", "2026-11-01T11:00:00Z"),
            spawn("w2", "2026-11-01T10:00:00Z", None, "claude"),
        ],
    );
    assert_eq!(labels(&view), ["unknown", "total"]);
    assert_eq!(row(&view, "unknown")[1], "20");
}

#[test]
fn a_known_ticket_sorts_before_the_unknown_row() {
    let view = view_of(
        View::Ticket,
        &[
            turn(),
            worker("w1", "2026-11-01T10:30:00Z", "claude-opus-5-5"),
        ],
        &[spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1"), "claude")],
    );
    assert_eq!(labels(&view), ["GUZ-1", "unknown", "total"]);
}

#[test]
fn a_cursor_worker_with_no_usage_rows_is_a_fully_unknown_ticket_row() {
    let view = view_of(
        View::Ticket,
        &[worker("w1", "2026-11-01T10:30:00Z", "claude-opus-5-5")],
        &[
            spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1"), "claude"),
            spawn("w5", "2026-11-01T10:00:00Z", Some("GUZ-5"), "cursor"),
        ],
    );
    let cursor = row(&view, "GUZ-5");
    assert_eq!(cursor[1..], ["unknown"; 8]);
    assert!(!view
        .lines()
        .any(|line| line.starts_with("| GUZ-5 |") && line.contains(" 0 ")));
}

#[test]
fn a_claude_worker_with_no_usage_rows_is_not_listed() {
    let view = view_of(
        View::Ticket,
        &[turn()],
        &[spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1"), "claude")],
    );
    assert_eq!(labels(&view), ["unknown", "total"]);
}

#[test]
fn a_cursor_worker_without_a_ticket_makes_the_unknown_row_partial() {
    let view = view_of(
        View::Ticket,
        &[turn()],
        &[spawn("w5", "2026-11-01T10:00:00Z", None, "cursor")],
    );
    let unknown = row(&view, "unknown");
    assert_eq!(unknown[1..5], ["10+", "5+", "1.0k+", "100+"]);
    assert!(unknown[8].ends_with('+'));
}

#[test]
fn only_cursor_workers_are_unknown_in_the_role_and_harness_views() {
    let usage = [turn()];
    let workers = [spawn("w5", "2026-11-01T10:00:00Z", Some("GUZ-5"), "cursor")];
    let role = view_of(View::Role, &usage, &workers);
    assert_eq!(row(&role, "worker")[1..], ["unknown"; 8]);
    let harness = view_of(View::Harness, &usage, &workers);
    assert_eq!(row(&harness, "cursor")[1..], ["unknown"; 8]);
}

#[test]
fn a_cursor_row_is_not_matched_by_the_usage_of_another_window_with_the_same_id() {
    let view = view_of(
        View::Ticket,
        &[worker("w5", "2026-11-02T10:30:00Z", "claude-opus-5-5")],
        &[
            spawn("w5", "2026-11-01T10:00:00Z", Some("GUZ-5"), "cursor"),
            close("w5", "2026-11-01T11:00:00Z"),
            spawn("w5", "2026-11-02T10:00:00Z", Some("GUZ-6"), "cursor"),
        ],
    );
    assert_eq!(row(&view, "GUZ-5")[1], "unknown");
    assert_eq!(row(&view, "GUZ-6")[1], "3");
}

#[test]
fn the_total_row_is_partial_when_any_row_is_unknown() {
    let view = view_of(
        View::Ticket,
        &[worker("w1", "2026-11-01T10:30:00Z", "claude-opus-5-5")],
        &[
            spawn("w1", "2026-11-01T10:00:00Z", Some("GUZ-1"), "claude"),
            spawn("w5", "2026-11-01T10:00:00Z", Some("GUZ-5"), "cursor"),
        ],
    );
    let total = row(&view, "total");
    assert_eq!(total[1..5], ["3+", "7+", "400+", "50+"]);
    assert!(total[8].ends_with('+'));
}

#[test]
fn an_unpriced_model_shows_na_and_a_missing_tier_stays_unknown() {
    let mut legacy = worker("w1", "2026-11-01T10:30:00Z", "no-such-model");
    legacy.as_object_mut().unwrap().remove("cache_write_5m");
    let view = view_of(View::Role, &[legacy], &[]);
    let worker = row(&view, "worker");
    assert_eq!(worker[5], "–");
    assert_eq!(worker[8], "n/a");
}
