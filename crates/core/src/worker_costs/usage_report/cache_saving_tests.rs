use super::{saving_estimate, saving_lines, saving_rates, SavingRates, SAVING_RULE};
use crate::wire::price::PriceOverride;
use crate::wire::provider::{provider, ProviderId};
use crate::wire::view::Pricing;
use crate::worker_costs::builtin_prices;
use crate::worker_costs::usage_report::cache_periods::{IdlePeriod, PeriodOutcome};
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;

fn base() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-10T10:00:00Z")
        .map(|at| at.with_timezone(&Utc))
        .unwrap()
}

fn period(
    ping_after: i64,
    next_after: i64,
    outcome: PeriodOutcome,
    model: Option<&str>,
) -> IdlePeriod {
    IdlePeriod {
        run: None,
        pings: 1,
        ping_cost: 0.02,
        last_turn_at: Some(base()),
        last_ping_at: Some(base() + Duration::minutes(ping_after)),
        context_tokens: Some(100_000),
        model: model.map(str::to_string),
        next_turn_at: Some(base() + Duration::minutes(next_after)),
        outcome,
    }
}

fn close(left: f64, right: f64) {
    assert!((left - right).abs() < 1e-9, "left: {left} right: {right}");
}

fn rates(model: &str) -> Option<SavingRates> {
    (model == "m").then_some(SavingRates {
        write_per_mtok: 10.0,
        read_per_mtok: 1.0,
    })
}

fn counted(ping_after: i64, next_after: i64, outcome: PeriodOutcome) -> usize {
    saving_estimate(&[period(ping_after, next_after, outcome, Some("m"))], rates).counted
}

#[test]
fn rule_counts_inside_both_bounds() {
    assert_eq!(counted(2, 61, PeriodOutcome::Warm), 1);
}

#[test]
fn rule_excludes_exactly_sixty_minutes_after_last_ping() {
    assert_eq!(counted(10, 70, PeriodOutcome::Warm), 0);
    assert_eq!(counted(10, 69, PeriodOutcome::Warm), 1);
}

#[test]
fn rule_excludes_exactly_sixty_minutes_after_last_turn() {
    assert_eq!(counted(30, 60, PeriodOutcome::Warm), 0);
}

#[test]
fn cold_and_no_next_turn_never_count() {
    assert_eq!(counted(2, 61, PeriodOutcome::Cold), 0);
    assert_eq!(counted(2, 61, PeriodOutcome::NoNextTurn), 0);
}

#[test]
fn missing_timestamp_or_context_does_not_count() {
    let mut no_next = period(2, 61, PeriodOutcome::Warm, Some("m"));
    no_next.next_turn_at = None;
    let mut no_ping = period(2, 61, PeriodOutcome::Warm, Some("m"));
    no_ping.last_ping_at = None;
    let mut no_turn = period(2, 61, PeriodOutcome::Warm, Some("m"));
    no_turn.last_turn_at = None;
    let mut no_context = period(2, 61, PeriodOutcome::Warm, Some("m"));
    no_context.context_tokens = None;
    let estimate = saving_estimate(&[no_next, no_ping, no_turn, no_context], rates);
    assert_eq!(estimate.counted, 0);
}

#[test]
fn saving_is_context_times_rate_gap_minus_ping_cost() {
    let estimate = saving_estimate(&[period(2, 61, PeriodOutcome::Warm, Some("m"))], rates);
    assert!((estimate.saved.unwrap() - 0.88).abs() < 1e-9);
    assert_eq!(estimate.counted, 1);
    assert_eq!(estimate.unpriced, 0);
    assert_eq!(estimate.rates_used.len(), 1);
    assert_eq!(estimate.rates_used[0].0, "m");
}

#[test]
fn saving_is_net_of_ping_cost_of_uncounted_periods() {
    let mut counted_period = period(2, 61, PeriodOutcome::Warm, Some("m"));
    counted_period.pings = 1;
    let mut cold = period(2, 61, PeriodOutcome::Cold, Some("m"));
    cold.ping_cost = 0.5;
    let estimate = saving_estimate(&[counted_period, cold], rates);
    close(estimate.saved.unwrap(), 0.9 - 0.02 - 0.5);
    assert_eq!(estimate.counted, 1);
    close(estimate.ping_cost, 0.52);
}

#[test]
fn lone_uncounted_ping_gives_no_figure() {
    let mut cold = period(2, 61, PeriodOutcome::Cold, Some("m"));
    cold.ping_cost = 0.5;
    assert_eq!(saving_estimate(&[cold], rates).saved, None);
}

#[test]
fn savings_sum_and_negative_terms_stay() {
    let mut small = period(2, 61, PeriodOutcome::Warm, Some("m"));
    small.context_tokens = Some(1_000);
    small.ping_cost = 0.5;
    let big = period(2, 61, PeriodOutcome::Warm, Some("m"));
    let estimate = saving_estimate(&[big, small], rates);
    assert!((estimate.saved.unwrap() - (0.9 + 0.009 - 0.02 - 0.5)).abs() < 1e-9);
}

#[test]
fn unpriced_model_gives_no_saved_value() {
    let estimate = saving_estimate(
        &[
            period(2, 61, PeriodOutcome::Warm, Some("other")),
            period(2, 61, PeriodOutcome::Warm, None),
        ],
        rates,
    );
    assert_eq!((estimate.counted, estimate.unpriced), (2, 2));
    assert_eq!(estimate.saved, None);
    assert!(estimate.rates_used.is_empty());
    let lines = saving_lines(&estimate);
    assert_eq!(lines[0], "saving estimate (est.): no price");
    assert!(lines.contains(&"2 period(s) without a price not counted".to_string()));
}

#[test]
fn mixed_periods_sum_the_priced_ones_and_report_the_rest() {
    let estimate = saving_estimate(
        &[
            period(2, 61, PeriodOutcome::Warm, Some("m")),
            period(2, 61, PeriodOutcome::Warm, Some("other")),
        ],
        rates,
    );
    close(estimate.saved.unwrap(), 0.9 - 0.04);
    assert_eq!(estimate.unpriced, 1);
}

#[test]
fn lines_show_total_rule_and_rates() {
    let estimate = saving_estimate(&[period(2, 61, PeriodOutcome::Warm, Some("m"))], rates);
    let lines = saving_lines(&estimate);
    assert_eq!(
        lines[0],
        "saving estimate (est.): $0.88 net of $0.02 ping cost, over 1 periods"
    );
    assert_eq!(lines[1], format!("rule: {SAVING_RULE}"));
    assert_eq!(
        lines[2],
        "write price m: $10.00/MTok (1h tier), read $1.00/MTok"
    );
    assert_eq!(lines.len(), 3);
}

#[test]
fn rates_used_are_sorted_by_model() {
    let both = |model: &str| {
        Some(SavingRates {
            write_per_mtok: if model == "b" { 5.0 } else { 6.0 },
            read_per_mtok: 1.0,
        })
    };
    let estimate = saving_estimate(
        &[
            period(2, 61, PeriodOutcome::Warm, Some("b")),
            period(2, 61, PeriodOutcome::Warm, Some("a")),
            period(2, 61, PeriodOutcome::Warm, Some("b")),
        ],
        both,
    );
    let models: Vec<&str> = estimate
        .rates_used
        .iter()
        .map(|(model, _)| model.as_str())
        .collect();
    assert_eq!(models, ["a", "b"]);
}

#[test]
fn known_model_has_write_above_read_above_zero() {
    let overrides = BTreeMap::new();
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    let table = builtin_prices(provider.as_ref());
    let found = saving_rates("claude-opus-5-5", &pricing, &table).unwrap();
    assert!(found.write_per_mtok > found.read_per_mtok);
    assert!(found.read_per_mtok > 0.0);
}

#[test]
fn override_changes_the_rates() {
    let mut overrides = BTreeMap::new();
    overrides.insert(
        "claude-opus-5-5".to_string(),
        PriceOverride {
            input: 4.0,
            cache_read: Some(0.4),
            cache_write_1h: Some(8.0),
            ..PriceOverride::default()
        },
    );
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    let table = builtin_prices(provider.as_ref());
    let found = saving_rates("claude-opus-5-5", &pricing, &table).unwrap();
    assert!((found.write_per_mtok - 8.0).abs() < 1e-9);
    assert!((found.read_per_mtok - 0.4).abs() < 1e-9);
}

#[test]
fn unknown_model_has_no_rates() {
    let overrides = BTreeMap::new();
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &overrides,
    };
    let table = builtin_prices(provider.as_ref());
    assert_eq!(saving_rates("no-such-model-9", &pricing, &table), None);
}
