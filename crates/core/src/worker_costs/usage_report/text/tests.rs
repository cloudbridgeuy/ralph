use super::*;
use crate::wire::price::PriceOverride;
use crate::wire::provider::{provider, ProviderId};
use crate::worker_costs::usage_report::{parse_bound, Dim};
use crate::worker_costs::{Usd, NO_USAGE};
use chrono::{FixedOffset, Utc};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn jsonl(rows: &[Value]) -> String {
    rows.iter().map(|row| format!("{row}\n")).collect()
}

fn row(kind: &str, at: &str) -> Value {
    json!({
        "kind": kind, "at": at, "harness": "claude", "model": "claude-haiku-5-5",
        "input_tokens": 100, "output_tokens": 10,
        "cache_creation_input_tokens": 50, "cache_read_input_tokens": 400,
        "cost_source": "harness", "cost_usd": 2.5,
    })
}

fn with(mut value: Value, key: &str, field: Value) -> Value {
    value[key] = field;
    value
}

fn worker(id: &str, at: &str) -> Value {
    with(
        with(row("worker", at), "worker_id", json!(id)),
        "session_id",
        json!(id),
    )
}

fn spawn(id: &str, ticket: &str, harness: &str) -> Value {
    json!({"at": "2026-11-01T00:00:00Z", "worker_id": id, "event": "spawn", "ticket": ticket, "harness": harness})
}

fn shown(view: View, rows: &[Value], workers: &[Value]) -> String {
    shown_raw(view, &jsonl(rows), workers)
}

fn shown_raw(view: View, usage: &str, workers: &[Value]) -> String {
    let overrides: BTreeMap<String, PriceOverride> = BTreeMap::new();
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    let inputs = Inputs {
        usage,
        workers: &jsonl(workers),
    };
    render(view, &inputs, &Filter::default(), &Utc, &pricing)
}

fn group(key: &str, reported: Option<f64>, computed: Option<f64>) -> Group {
    Group {
        key: key.to_string(),
        metered: 1,
        reported: reported.map_or_else(Usd::default, Usd::reported),
        est: computed.map_or_else(Usd::default, |usd| Usd {
            known: Some(usd),
            estimate: true,
            ..Usd::default()
        }),
        ..Group::default()
    }
}

fn unpriced(key: &str) -> Group {
    Group {
        key: key.to_string(),
        metered: 3,
        est: Usd {
            missing: true,
            estimate: true,
            ..Usd::default()
        },
        ..Group::default()
    }
}

#[test]
fn an_unpriced_group_line_reads_no_price_never_na_or_zero() {
    let line = unpriced("(model missing)").line();
    assert_eq!(line, "(model missing): no price");
    assert!(!line.contains("n/a") && !line.contains("$0"), "{line}");
}

#[test]
fn a_group_line_carries_dollars_and_est_only_for_computed() {
    assert_eq!(
        group("a", Some(5.0), Some(2.0)).line(),
        "a $5.00 + $2.00 est."
    );
    assert_eq!(group("a", Some(5.0), None).line(), "a $5.00");
    assert_eq!(group("a", None, Some(2.0)).line(), "a $2.00 est.");
}

#[test]
fn the_harness_table_labels_cursor_and_codex() {
    let at = "2026-11-05T10:00:00Z";
    let rows = [
        worker("w1", at),
        with(worker("w2", at), "harness", json!("codex")),
    ];
    let workers = [
        spawn("w1", "GUZ-1", "claude"),
        spawn("w2", "GUZ-2", "codex"),
        spawn("w3", "GUZ-3", "cursor"),
    ];
    let text = shown(View::By(Dim::Harness), &rows, &workers);
    assert!(text.contains("cursor: no data"), "{text}");
    assert!(text.contains("codex: excluded"), "{text}");
    assert!(text.contains("$2.50"), "{text}");
    assert!(!text.contains("n/a") && !text.contains("$0"), "{text}");
}

#[test]
fn the_model_table_labels_a_missing_model_with_no_price() {
    let rows = [with(
        with(row("ask", "2026-11-05T10:00:00Z"), "model", Value::Null),
        "cost_usd",
        Value::Null,
    )];
    let text = shown(View::By(Dim::Model), &rows, &[]);
    assert!(text.contains("(model missing): no price"), "{text}");
    assert!(!text.contains("n/a") && !text.contains("$0"), "{text}");
}

#[test]
fn a_mixed_group_reads_reported_plus_est() {
    let at = "2026-11-05T10:00:00Z";
    let priced = with(
        with(worker("w1", at), "cost_usd", Value::Null),
        "cost_source",
        Value::Null,
    );
    let rows = [worker("w1", at), priced];
    let text = shown(
        View::By(Dim::Ticket),
        &rows,
        &[spawn("w1", "GUZ-1", "claude")],
    );
    assert!(text.contains("GUZ-1"), "{text}");
    assert!(text.contains("$2.50 + $"), "{text}");
    assert!(text.contains(" est."), "{text}");
}

#[test]
fn the_sparkline_scales_gaps_and_unpriced_days() {
    let days = [
        group("2026-11-05", Some(1.0), None),
        Group {
            key: "2026-11-06".to_string(),
            ..Group::default()
        },
        unpriced("2026-11-07"),
        group("2026-11-08", Some(4.0), None),
    ];
    assert_eq!(sparkline(&days), "▃ ?█");
}

#[test]
fn the_daily_text_has_a_trend_line_and_a_row_per_used_day() {
    let rows = [
        row("ask", "2026-11-05T10:00:00Z"),
        row("ask", "2026-11-07T10:00:00Z"),
    ];
    let text = shown(View::Daily, &rows, &[]);
    assert!(text.contains("trend"), "{text}");
    assert!(
        text.contains("2026-11-05") && text.contains("2026-11-07"),
        "{text}"
    );
    assert!(!text.contains("2026-11-06"), "{text}");
}

#[test]
fn the_cache_text_shows_the_hit_rate() {
    let text = shown(View::Cache, &[row("ask", "2026-11-05T10:00:00Z")], &[]);
    assert!(text.contains("hit 73%"), "{text}");
}

#[test]
fn malformed_lines_are_counted_in_the_text() {
    let usage = format!(
        "{}not json\nalso bad\n",
        jsonl(&[row("ask", "2026-11-05T10:00:00Z")])
    );
    let text = shown_raw(View::By(Dim::Kind), &usage, &[]);
    assert!(text.contains("malformed lines skipped: 2"), "{text}");
}

#[test]
fn a_clean_ledger_prints_no_malformed_line() {
    let text = shown(
        View::By(Dim::Kind),
        &[row("ask", "2026-11-05T10:00:00Z")],
        &[],
    );
    assert!(!text.contains("malformed"), "{text}");
}

#[test]
fn an_empty_ledger_says_no_usage() {
    let text = shown(View::By(Dim::Kind), &[], &[]);
    assert!(text.contains(NO_USAGE), "{text}");
}

#[test]
fn a_bound_is_a_local_day_or_an_instant() {
    let zone = FixedOffset::east_opt(2 * 3600).unwrap();
    let day = parse_bound("2026-11-05", &zone).map(|at| at.to_rfc3339());
    assert_eq!(day.as_deref(), Some("2026-11-04T22:00:00+00:00"));
    let instant = parse_bound("2026-11-05T10:00:00Z", &zone).map(|at| at.to_rfc3339());
    assert_eq!(instant.as_deref(), Some("2026-11-05T10:00:00+00:00"));
    assert!(parse_bound("yesterday", &zone).is_none());
}
