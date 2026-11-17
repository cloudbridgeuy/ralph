use super::cache_periods::{outcome_counts, IdlePeriod, PeriodOutcome};
use crate::chat_process::{UsageLine, CACHE_TTL_SECS};
use crate::wire::price::PriceTable;
use crate::wire::view::Pricing;
use crate::worker_costs::{computed_usd, MAIN_HARNESS};
use std::collections::BTreeMap;

pub const SAVING_RULE: &str =
    "next turn < 60 min after last ping and > 60 min after last real turn";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavingRates {
    pub write_per_mtok: f64,
    pub read_per_mtok: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavingEstimate {
    pub counted: usize,
    pub unpriced: usize,
    pub saved: Option<f64>,
    pub ping_cost: f64,
    pub rates_used: Vec<(String, SavingRates)>,
}

fn probe(model: &str, write: bool) -> UsageLine {
    let tokens = 1_000_000;
    let base = UsageLine {
        model: Some(model.to_string()),
        harness: Some(MAIN_HARNESS.to_string()),
        long_prompt: Some(false),
        ..UsageLine::default()
    };
    if write {
        UsageLine {
            cache_creation_input_tokens: tokens,
            cache_write_1h: Some(tokens),
            cache_write_5m: Some(0),
            ..base
        }
    } else {
        UsageLine {
            cache_read_input_tokens: tokens,
            ..base
        }
    }
}

fn per_mtok(model: &str, write: bool, pricing: &Pricing, table: &PriceTable) -> Option<f64> {
    let usd = computed_usd(&probe(model, write), pricing, table);
    if usd.missing {
        None
    } else {
        usd.known
    }
}

pub fn saving_rates(model: &str, pricing: &Pricing, table: &PriceTable) -> Option<SavingRates> {
    Some(SavingRates {
        write_per_mtok: per_mtok(model, true, pricing, table)?,
        read_per_mtok: per_mtok(model, false, pricing, table)?,
    })
}

struct Counted<'a> {
    context: u64,
    model: Option<&'a str>,
}

fn counted(period: &IdlePeriod) -> Option<Counted<'_>> {
    let (last_turn, last_ping, next) = (
        period.last_turn_at?,
        period.last_ping_at?,
        period.next_turn_at?,
    );
    let saved = period.outcome == PeriodOutcome::Warm
        && (next - last_ping).num_seconds() < CACHE_TTL_SECS
        && (next - last_turn).num_seconds() > CACHE_TTL_SECS;
    saved.then(|| Counted {
        context: period.context_tokens.unwrap_or(0),
        model: period.model.as_deref(),
    })
}

pub fn saving_estimate(
    periods: &[IdlePeriod],
    rates: impl Fn(&str) -> Option<SavingRates>,
) -> SavingEstimate {
    let mut used: BTreeMap<String, SavingRates> = BTreeMap::new();
    let mut saved: Option<f64> = None;
    let mut count = 0;
    let mut unpriced = 0;
    for period in periods
        .iter()
        .filter(|period| period.context_tokens.is_some())
    {
        let Some(item) = counted(period) else {
            continue;
        };
        count += 1;
        let Some((model, rate)) = item.model.and_then(|model| Some((model, rates(model)?))) else {
            unpriced += 1;
            continue;
        };
        used.insert(model.to_string(), rate);
        let gain = item.context as f64 / 1e6 * (rate.write_per_mtok - rate.read_per_mtok);
        saved = Some(saved.unwrap_or(0.0) + gain);
    }
    let ping_cost = outcome_counts(periods).ping_cost;
    SavingEstimate {
        counted: count,
        unpriced,
        saved: saved.map(|gross| gross - ping_cost),
        ping_cost,
        rates_used: used.into_iter().collect(),
    }
}

pub fn saving_lines(estimate: &SavingEstimate) -> Vec<String> {
    let first = match estimate.saved {
        Some(saved) => format!(
            "saving estimate (est.): ${saved:.2} net of ${:.2} ping cost, over {} periods",
            estimate.ping_cost,
            estimate.counted - estimate.unpriced
        ),
        None if estimate.counted == 0 => "saving estimate (est.): no qualifying period".to_string(),
        None => "saving estimate (est.): no price".to_string(),
    };
    let mut lines = vec![first, format!("rule: {SAVING_RULE}")];
    lines.extend(estimate.rates_used.iter().map(|(model, rate)| {
        format!(
            "write price {model}: ${:.2}/MTok (1h tier), read ${:.2}/MTok",
            rate.write_per_mtok, rate.read_per_mtok
        )
    }));
    if estimate.unpriced > 0 {
        lines.push(format!(
            "{} period(s) without a price not counted",
            estimate.unpriced
        ));
    }
    lines
}

#[cfg(test)]
#[path = "cache_saving_tests.rs"]
mod cache_saving_tests;
