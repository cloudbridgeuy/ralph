use super::*;
use crate::chat_process::UsageLine;
use crate::wire::price::PriceOverride;
use crate::wire::provider::{provider, ProviderId};
use crate::worker_costs::tickets::parse_windows;
use crate::worker_costs::window::instant;
use chrono::FixedOffset;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn jsonl(rows: &[Value]) -> String {
    rows.iter().map(|row| format!("{row}\n")).collect()
}

fn row(kind: &str, at: &str) -> Value {
    json!({
        "kind": kind, "at": at, "harness": "claude", "model": "claude-haiku-5-5",
        "input_tokens": 100, "output_tokens": 10,
        "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0,
    })
}

fn worker(id: &str, at: &str) -> Value {
    let mut value = row("worker", at);
    value["worker_id"] = json!(id);
    value["session_id"] = json!(id);
    value
}

fn with(mut value: Value, key: &str, field: Value) -> Value {
    value[key] = field;
    value
}

fn spawn(id: &str, at: &str, ticket: &str, harness: &str) -> Value {
    json!({"at": at, "worker_id": id, "event": "spawn", "ticket": ticket, "harness": harness})
}

fn ledger(rows: &[Value], workers: &[Value]) -> (Ledger, Vec<WorkerWindow>) {
    let windows = parse_windows(&jsonl(workers));
    (parse(&jsonl(rows), &windows), windows)
}

fn grouped(dim: Dim, rows: &[Value], workers: &[Value], filter: &Filter) -> Report {
    let (ledger, windows) = ledger(rows, workers);
    let overrides: BTreeMap<String, PriceOverride> = BTreeMap::new();
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    group_by(dim, &ledger, &windows, filter, &pricing)
}

fn keys(report: &Report) -> Vec<&str> {
    report
        .groups
        .iter()
        .map(|group| group.key.as_str())
        .collect()
}

fn known(usd: Usd) -> Option<f64> {
    usd.known
}

#[test]
fn malformed_lines_are_counted_and_skipped() {
    let text = format!(
        "{}\nnot json\n{}\n{}\n\n{}\n",
        row("ask", "2026-11-05T10:00:00Z"),
        json!({"kind": "ask", "at": "yesterday"}),
        json!({"kind": "ask"}),
        row("mystery", "2026-11-05T10:00:00Z"),
    );
    let ledger = parse(&text, &[]);
    assert_eq!((ledger.entries.len(), ledger.malformed), (1, 3));
}

#[test]
fn the_report_carries_the_malformed_count() {
    let report = grouped(
        Dim::Kind,
        &[row("ask", "2026-11-05T10:00:00Z"), json!({"kind": "ask"})],
        &[],
        &Filter::default(),
    );
    assert_eq!(report.malformed, 1);
}

#[test]
fn since_is_inclusive_and_until_is_exclusive() {
    let rows = [
        row("ask", "2026-11-05T10:00:00Z"),
        row("ask", "2026-11-05T11:00:00Z"),
        row("ask", "2026-11-05T12:00:00Z"),
    ];
    let filter = Filter {
        since: instant("2026-11-05T11:00:00Z"),
        until: instant("2026-11-05T12:00:00Z"),
        ..Filter::default()
    };
    let report = grouped(Dim::Kind, &rows, &[], &filter);
    assert_eq!(report.total.metered, 1);
}

#[test]
fn filters_narrow_by_store_harness_model_kind_and_ticket() {
    let at = "2026-11-05T10:00:00Z";
    let rows = [
        with(row("ask", at), "store", json!("a")),
        with(
            with(row("nap", at), "store", json!("b")),
            "model",
            json!("m2"),
        ),
        worker("w1", at),
        with(worker("w2", at), "harness", json!("codex")),
    ];
    let workers = [spawn("w1", "2026-11-05T09:00:00Z", "GUZ-1", "claude")];
    let count = |filter: Filter| grouped(Dim::Kind, &rows, &workers, &filter).total.metered;
    let by = |f: &dyn Fn(&mut Filter)| {
        let mut filter = Filter::default();
        f(&mut filter);
        count(filter)
    };
    assert_eq!(by(&|f| f.store = Some("a".into())), 1);
    assert_eq!(by(&|f| f.harness = Some("codex".into())), 1);
    assert_eq!(by(&|f| f.harness = Some("nap".into())), 1);
    assert_eq!(by(&|f| f.model = Some("m2".into())), 1);
    assert_eq!(by(&|f| f.kind = Some("worker".into())), 2);
    assert_eq!(by(&|f| f.ticket = Some("GUZ-1".into())), 1);
    assert_eq!(by(&|f| f.ticket = Some("unknown".into())), 3);
}

#[test]
fn groups_by_every_dimension() {
    let at = "2026-11-05T10:00:00Z";
    let rows = [
        with(
            with(row("turn", at), "run", json!("r1")),
            "store",
            json!("s1"),
        ),
        with(
            with(row("nap", at), "model", Value::Null),
            "run",
            json!("r2"),
        ),
        worker("w1", at),
    ];
    let workers = [spawn("w1", "2026-11-05T09:00:00Z", "GUZ-1", "claude")];
    let of = |dim| keys(&grouped(dim, &rows, &workers, &Filter::default())).join(",");
    assert_eq!(of(Dim::Kind), "main,nap,worker");
    assert_eq!(of(Dim::Harness), "main,nap,claude");
    assert_eq!(of(Dim::Store), "s1,unknown");
    assert_eq!(of(Dim::Model), "(model missing),claude-haiku-5-5");
    assert_eq!(of(Dim::Ticket), "GUZ-1,unknown");
    assert_eq!(of(Dim::Run), "r1,r2,unknown");
}

#[test]
fn computed_cost_is_its_own_est_series() {
    let report = grouped(
        Dim::Kind,
        &[worker("w1", "2026-11-05T10:00:00Z")],
        &[],
        &Filter::default(),
    );
    let group = &report.groups[0];
    assert!(known(group.est).is_some_and(|usd| usd > 0.0));
    assert_eq!(known(group.reported), None);
    assert!(group.line().contains("est."), "{}", group.line());
}

#[test]
fn harness_reported_cost_is_not_labelled_est() {
    let reported = with(
        with(worker("w1", "2026-11-05T10:00:00Z"), "cost_usd", json!(0.5)),
        "cost_source",
        json!("harness"),
    );
    let report = grouped(Dim::Kind, &[reported], &[], &Filter::default());
    let group = &report.groups[0];
    assert_eq!(known(group.reported), Some(0.5));
    assert_eq!(known(group.est), None);
    assert!(!group.line().contains("est."), "{}", group.line());
    assert!(group.line().contains("0.50"), "{}", group.line());
}

#[test]
fn a_missing_model_is_labelled_in_the_group_line() {
    let rows = [with(
        row("ask", "2026-11-05T10:00:00Z"),
        "model",
        Value::Null,
    )];
    let report = grouped(Dim::Model, &rows, &[], &Filter::default());
    assert!(report.groups[0].line().starts_with("(model missing)"));
}

#[test]
fn an_openai_prefixed_opencode_row_is_an_estimate_never_reported() {
    let rows = [with(
        with(
            worker("w1", "2026-11-05T10:00:00Z"),
            "harness",
            json!("opencode"),
        ),
        "model",
        json!("openai/gpt-6.1-sol"),
    )];
    let report = grouped(Dim::Model, &rows, &[], &Filter::default());
    let group = &report.groups[0];
    assert_eq!(group.key, "gpt-6.1-sol");
    assert!(known(group.est).is_some());
    assert_eq!(known(group.reported), None);
    assert!(
        group.cost_label().ends_with("est."),
        "{}",
        group.cost_label()
    );
}

#[test]
fn a_codex_gpt_6_1_sol_row_stays_excluded_from_dollars() {
    let rows = [with(
        with(
            worker("w1", "2026-11-05T10:00:00Z"),
            "harness",
            json!("codex"),
        ),
        "model",
        json!("gpt-6.1-sol"),
    )];
    let report = grouped(Dim::Model, &rows, &[], &Filter::default());
    let group = &report.groups[0];
    assert_eq!(group.excluded, 1);
    assert_eq!(known(group.est), None);
    assert_eq!(known(group.reported), None);
    assert_eq!(group.line(), "gpt-6.1-sol: excluded");
}

#[test]
fn a_marked_codex_row_joins_the_dollar_totals() {
    let at = "2026-11-05T10:00:00Z";
    let marked = with(
        with(
            with(worker("w1", at), "harness", json!("codex")),
            "model",
            json!("gpt-6.1-sol"),
        ),
        "per_thread_usage",
        json!(true),
    );
    let report = grouped(Dim::Model, &[marked], &[], &Filter::default());
    let group = &report.groups[0];
    assert_eq!(group.excluded, 0);
    assert!(known(group.est).is_some());
    assert_ne!(group.line(), "gpt-6.1-sol: excluded");
}

#[test]
fn a_cursor_with_no_rows_shows_no_data_never_zero() {
    let workers = [spawn("c1", "2026-11-05T09:00:00Z", "GUZ-2", "cursor")];
    let report = grouped(Dim::Harness, &[], &workers, &Filter::default());
    assert_eq!(report.groups[0].line(), "cursor: no data");
    assert!(!report.groups[0].line().contains("0.00"));
}

#[test]
fn codex_tokens_count_but_its_dollars_stay_out_of_the_totals() {
    let at = "2026-11-05T10:00:00Z";
    let rows = [
        with(
            with(worker("w1", at), "harness", json!("codex")),
            "model",
            json!("gpt-5"),
        ),
        worker("w2", at),
    ];
    let report = grouped(Dim::Harness, &rows, &[], &Filter::default());
    let codex = report.groups.iter().find(|g| g.key == "codex").unwrap();
    assert_eq!((codex.tokens.input, codex.excluded), (100, 1));
    assert_eq!(known(codex.est), None);
    let claude = report.groups.iter().find(|g| g.key == "claude").unwrap();
    assert_eq!(known(report.total.est), known(claude.est));
    assert_eq!(report.total.tokens.input, 200);
}

#[test]
fn daily_buckets_use_the_local_day() {
    let rows = [
        row("ask", "2026-11-05T21:00:00Z"),
        row("ask", "2026-11-05T23:30:00Z"),
        row("ask", "2026-11-06T00:30:00Z"),
    ];
    let (ledger, windows) = ledger(&rows, &[]);
    let overrides: BTreeMap<String, PriceOverride> = BTreeMap::new();
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    let zone = FixedOffset::east_opt(2 * 3600).unwrap();
    let report = daily(&ledger, &windows, &Filter::default(), &zone, &pricing);
    let days: Vec<_> = report
        .groups
        .iter()
        .map(|g| (g.key.as_str(), g.metered))
        .collect();
    assert_eq!(days, [("2026-11-05", 1), ("2026-11-06", 2)]);
}

#[test]
fn cache_hit_rate_is_reads_over_all_prompt_tokens() {
    let mut value = row("ask", "2026-11-05T10:00:00Z");
    value["input_tokens"] = json!(50);
    value["cache_creation_input_tokens"] = json!(50);
    value["cache_read_input_tokens"] = json!(900);
    let report = grouped(Dim::Model, &[value], &[], &Filter::default());
    let rate = report.groups[0].cache_hit_rate().unwrap();
    assert!((rate - 0.9).abs() < 1e-9);
    assert_eq!(Group::default().cache_hit_rate(), None);
}

#[test]
fn the_usage_line_reads_an_optional_store() {
    let with_store: UsageLine = serde_json::from_str(r#"{"store":"s1"}"#).unwrap();
    let without: UsageLine = serde_json::from_str("{}").unwrap();
    assert_eq!(
        (with_store.store.as_deref(), without.store),
        (Some("s1"), None)
    );
}

#[test]
fn a_codex_group_line_says_excluded_never_a_price() {
    let rows = [with(
        with(
            worker("w1", "2026-11-05T10:00:00Z"),
            "harness",
            json!("codex"),
        ),
        "model",
        json!("gpt-5"),
    )];
    let report = grouped(Dim::Harness, &rows, &[], &Filter::default());
    assert_eq!(report.groups[0].line(), "codex: excluded");
}

#[test]
fn a_mixed_group_line_keeps_the_known_dollars_and_flags_the_excluded_rows() {
    let at = "2026-11-05T10:00:00Z";
    let rows = [
        with(worker("w1", at), "harness", json!("codex")),
        worker("w2", at),
    ];
    let report = grouped(Dim::Kind, &rows, &[], &Filter::default());
    let line = report.groups[0].line();
    assert!(
        line.contains("est.") && line.ends_with("+ excluded"),
        "{line}"
    );
}

#[test]
fn a_local_day_names_its_english_weekday() {
    assert_eq!(weekday("2026-11-05"), Some("Thu"));
    assert_eq!(weekday("2026-11-08"), Some("Sun"));
    assert_eq!(weekday("2026-11-09"), Some("Mon"));
    assert_eq!(weekday("2026-11-10"), Some("Tue"));
}

#[test]
fn text_that_is_not_a_day_has_no_weekday() {
    assert_eq!(weekday("total"), None);
    assert_eq!(weekday("2026-13-40"), None);
    assert_eq!(weekday(""), None);
}

fn model_row(harness: &str, model: &str) -> Value {
    with(
        with(
            row("ask", "2026-11-05T10:00:00Z"),
            "harness",
            json!(harness),
        ),
        "model",
        json!(model),
    )
}

fn sol_rows() -> [Value; 2] {
    [
        model_row("opencode", "openai/gpt-6.1-sol"),
        model_row("codex", "gpt-6.1-sol"),
    ]
}

#[test]
fn an_openai_prefixed_model_and_its_bare_id_share_one_model_group() {
    let report = grouped(Dim::Model, &sol_rows(), &[], &Filter::default());
    assert_eq!(keys(&report), ["gpt-6.1-sol"]);
    let group = &report.groups[0];
    assert_eq!((group.metered, group.excluded), (2, 1));
    assert!(known(group.est).is_some());
}

#[test]
fn a_merged_model_group_shows_the_estimate_and_the_excluded_part() {
    let report = grouped(Dim::Model, &sol_rows(), &[], &Filter::default());
    let line = report.groups[0].line();
    assert!(
        line.starts_with("gpt-6.1-sol ") && line.contains("est.") && line.ends_with("+ excluded"),
        "{line}"
    );
}

#[test]
fn the_model_filter_matches_the_prefixed_and_the_bare_id_alike() {
    for wanted in ["gpt-6.1-sol", "openai/gpt-6.1-sol"] {
        let filter = Filter {
            model: Some(wanted.into()),
            ..Filter::default()
        };
        let report = grouped(Dim::Model, &sol_rows(), &[], &filter);
        assert_eq!(report.total.metered, 2, "{wanted}");
    }
}

#[test]
fn other_provider_prefixes_stay_separate_model_groups() {
    let rows = [
        model_row("opencode", "github-copilot/gpt-6.1-sol"),
        model_row("codex", "gpt-6.1-sol"),
    ];
    let report = grouped(Dim::Model, &rows, &[], &Filter::default());
    assert_eq!(keys(&report), ["github-copilot/gpt-6.1-sol", "gpt-6.1-sol"]);
}

#[test]
fn a_bracketed_model_variant_stays_apart_from_its_bare_id() {
    let rows = [
        model_row("claude", "claude-opus-5-5[1m]"),
        model_row("claude", "claude-opus-5-5"),
    ];
    let report = grouped(Dim::Model, &rows, &[], &Filter::default());
    assert_eq!(keys(&report), ["claude-opus-5-5", "claude-opus-5-5[1m]"]);
}
