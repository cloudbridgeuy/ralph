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

fn partial(key: &str, reported: Option<f64>, computed: Option<f64>) -> Group {
    let mut group = group(key, reported, computed);
    group.est.missing = true;
    group
}

fn plus_after_a_digit(text: &str) -> bool {
    text.chars()
        .zip(text.chars().skip(1))
        .any(|(before, after)| before.is_ascii_digit() && after == '+')
}

#[test]
fn a_partial_price_reads_no_price_never_a_trailing_plus() {
    assert_eq!(
        partial("a", None, Some(5.0)).line(),
        "a $5.00 est. + no price"
    );
    assert_eq!(
        partial("a", Some(5.0), Some(2.0)).line(),
        "a $5.00 + $2.00 est. + no price"
    );
    assert_eq!(partial("a", Some(5.0), None).line(), "a $5.00 + no price");
}

#[test]
fn the_total_row_of_a_partly_priced_table_reads_no_price() {
    let priced = row("ask", "2026-11-05T10:00:00Z");
    let unpriced = with(
        with(row("ask", "2026-11-05T11:00:00Z"), "model", Value::Null),
        "cost_usd",
        Value::Null,
    );
    let text = shown(View::By(Dim::Kind), &[priced, unpriced], &[]);
    assert!(text.contains("total"), "{text}");
    assert!(text.contains("+ no price"), "{text}");
    assert!(!plus_after_a_digit(&text), "{text}");
}

#[test]
fn the_sparkline_legend_shows_only_when_a_question_mark_does() {
    let unpriced = with(
        with(row("ask", "2026-11-05T10:00:00Z"), "model", Value::Null),
        "cost_usd",
        Value::Null,
    );
    let with_mark = shown(View::Daily, &[unpriced], &[]);
    assert!(
        with_mark.contains("? = day with rows but no price"),
        "{with_mark}"
    );
    let without = shown(View::Daily, &[row("ask", "2026-11-05T10:00:00Z")], &[]);
    assert!(!without.contains("? ="), "{without}");
}

#[test]
fn text_forces_text_even_on_a_terminal() {
    let asked = std::cell::Cell::new(false);
    let output = choose(View::Daily, true, None, || {
        asked.set(true);
        true
    });
    assert_eq!(output, Ok(Output::Text));
    assert!(!asked.get());
}

#[test]
fn a_png_path_wins_without_asking_the_terminal() {
    let asked = std::cell::Cell::new(false);
    let path = PathBuf::from("/tmp/x.png");
    let output = choose(View::By(Dim::Model), false, Some(path.clone()), || {
        asked.set(true);
        true
    });
    assert_eq!(output, Ok(Output::Png(path)));
    assert!(!asked.get());
}

#[test]
fn a_capable_terminal_draws_inline_and_anything_else_prints_text() {
    assert_eq!(
        choose(View::Daily, false, None, || true),
        Ok(Output::Inline)
    );
    assert_eq!(choose(View::Daily, false, None, || false), Ok(Output::Text));
}

#[test]
fn the_cache_view_has_no_chart() {
    assert_eq!(choose(View::Cache, false, None, || true), Ok(Output::Text));
    assert!(choose(
        View::Cache,
        false,
        Some(PathBuf::from("/tmp/x.png")),
        || true
    )
    .is_err());
}

fn chart_of(view: View, usage: &str) -> Option<String> {
    let overrides: BTreeMap<String, PriceOverride> = BTreeMap::new();
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    let inputs = Inputs { usage, workers: "" };
    chart(view, &inputs, &Filter::default(), &Utc, &pricing)
}

#[test]
fn the_chart_is_an_svg_titled_by_its_dimension() {
    let usage = jsonl(&[row("ask", "2026-11-05T10:00:00Z")]);
    let svg = chart_of(View::By(Dim::Harness), &usage).unwrap_or_default();
    assert!(svg.starts_with("<svg"), "{svg}");
    assert!(svg.contains(">Spend by harness<"), "{svg}");
    let daily = chart_of(View::Daily, &usage).unwrap_or_default();
    assert!(daily.contains(">Daily spend<"), "{daily}");
    assert!(chart_of(View::Cache, &usage).is_none());
}

#[test]
fn the_chart_subtitle_counts_malformed_lines() {
    let usage = format!("{}not json\n", jsonl(&[row("ask", "2026-11-05T10:00:00Z")]));
    let svg = chart_of(View::Daily, &usage).unwrap_or_default();
    assert!(svg.contains("malformed lines skipped: 1"), "{svg}");
}

fn at_store(value: Value, store: &str) -> Value {
    with(value, "store", json!(store))
}

#[test]
fn two_stores_are_read_and_both_their_tickets_join() {
    let usage = jsonl(&[
        at_store(worker("w-a", "2026-11-05T10:00:00Z"), "/s/b"),
        at_store(worker("w-b", "2026-11-05T11:00:00Z"), "/s/a"),
        at_store(row("ask", "2026-11-05T12:00:00Z"), "/s/a"),
        row("ask", "2026-11-05T13:00:00Z"),
    ]);
    assert_eq!(stores_to_read(&usage, None), ["/s/a", "/s/b"]);
    assert_eq!(stores_to_read(&usage, Some("/s/z")), ["/s/z"]);
    let text = shown_raw(
        View::By(Dim::Ticket),
        &usage,
        &[
            spawn("w-a", "GUZ-1", "claude"),
            spawn("w-b", "GUZ-2", "claude"),
        ],
    );
    assert!(text.contains("GUZ-1") && text.contains("GUZ-2"), "{text}");
}

#[test]
fn daily_rows_carry_the_weekday_after_the_date() {
    let text = shown(
        View::Daily,
        &[
            row("ask", "2026-11-05T10:00:00Z"),
            with(
                with(row("ask", "2026-11-09T10:00:00Z"), "model", Value::Null),
                "cost_usd",
                Value::Null,
            ),
        ],
        &[],
    );
    assert!(text.contains("2026-11-05 Thu  "), "{text}");
    assert!(text.contains("2026-11-09 Mon: no price"), "{text}");
    assert!(text.contains("\ntotal  "), "{text}");
}

#[test]
fn other_views_keep_plain_keys() {
    let text = shown(
        View::By(Dim::Kind),
        &[row("ask", "2026-11-05T10:00:00Z")],
        &[],
    );
    assert!(!text.contains("Thu"), "{text}");
}
