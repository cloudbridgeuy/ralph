use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::usage::{BillingRates, CacheTier, Usage, FIVE_MIN, ONE_HOUR};

#[derive(Debug, Clone, PartialEq)]
pub struct TierPrice {
    pub tier: CacheTier,
    pub usd_per_mtok: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelPrice {
    pub input: f64,
    pub cache_read: f64,
    pub cache_writes: Vec<TierPrice>,
    pub output: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PriceEntry {
    pub model: String,
    pub above_prompt_tokens: Option<u64>,
    pub price: ModelPrice,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PriceTable {
    pub as_of: &'static str,
    pub entries: Vec<PriceEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceOverride {
    pub input: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_5m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
}

pub fn normalize_model(model: &str) -> &str {
    model.split('[').next().unwrap_or(model)
}

/// A listed id matches exactly, or with one `-YYYYMMDD` suffix.
/// `claude-opus-5-6` does not match `claude-opus-5`.
pub fn matches_prefix(model: &str, prefix: &str) -> bool {
    model == prefix || model.strip_prefix(prefix).is_some_and(is_date_suffix)
}

fn is_date_suffix(rest: &str) -> bool {
    let Some(digits) = rest.strip_prefix('-') else {
        return false;
    };
    digits.len() == 8 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// Rejects a negative override price. Zero is allowed.
pub fn validate_price_overrides(overrides: &BTreeMap<String, PriceOverride>) -> Result<(), String> {
    for (prefix, price) in overrides {
        for (name, value) in [
            ("input", Some(price.input)),
            ("cache_read", price.cache_read),
            ("cache_write_5m", price.cache_write_5m),
            ("cache_write_1h", price.cache_write_1h),
            ("output", price.output),
        ] {
            if value.is_some_and(|amount| amount < 0.0) {
                return Err(format!(
                    "wire price for '{prefix}' {name} must be zero or positive"
                ));
            }
        }
    }
    Ok(())
}

pub fn prompt_tokens(usage: &Usage) -> u64 {
    usage.input_uncached
        + usage.cache_read
        + usage.cache_writes.iter().map(|w| w.tokens).sum::<u64>()
}

impl PriceTable {
    pub fn lookup(&self, model: &str, prompt: u64) -> Option<&ModelPrice> {
        self.entry(model, prompt).map(|entry| &entry.price)
    }

    fn entry(&self, model: &str, prompt: u64) -> Option<&PriceEntry> {
        let model = normalize_model(model);
        let longest = self
            .entries
            .iter()
            .filter(|entry| matches_prefix(model, &entry.model))
            .map(|entry| entry.model.len())
            .max()?;
        self.entries
            .iter()
            .filter(|entry| entry.model.len() == longest && matches_prefix(model, &entry.model))
            .filter(|entry| entry.above_prompt_tokens.is_none_or(|t| prompt > t))
            .max_by_key(|entry| entry.above_prompt_tokens)
    }
}

pub fn long_prompt(table: &PriceTable, model: &str, prompt: u64) -> bool {
    table
        .entry(model, prompt)
        .is_some_and(|entry| entry.above_prompt_tokens.is_some())
}

fn derived_write(
    explicit: Option<f64>,
    input: f64,
    tier: CacheTier,
    rates: &BillingRates,
) -> Option<TierPrice> {
    explicit
        .or_else(|| {
            rates
                .cache_writes
                .iter()
                .find(|rate| rate.tier == tier)
                .map(|rate| input * rate.multiplier)
        })
        .map(|usd_per_mtok| TierPrice { tier, usd_per_mtok })
}

impl PriceOverride {
    fn to_price(&self, rates: &BillingRates) -> ModelPrice {
        ModelPrice {
            input: self.input,
            cache_read: self.cache_read.unwrap_or(self.input * rates.cache_read),
            cache_writes: [
                derived_write(self.cache_write_5m, self.input, FIVE_MIN, rates),
                derived_write(self.cache_write_1h, self.input, ONE_HOUR, rates),
            ]
            .into_iter()
            .flatten()
            .collect(),
            output: self.output.unwrap_or(self.input * rates.output),
        }
    }
}

pub fn resolve_price(
    table: &PriceTable,
    overrides: &BTreeMap<String, PriceOverride>,
    rates: &BillingRates,
    model: &str,
    prompt: u64,
) -> Option<ModelPrice> {
    let normalized = normalize_model(model);
    overrides
        .iter()
        .filter(|(prefix, _)| matches_prefix(normalized, prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, over)| over.to_price(rates))
        .or_else(|| table.lookup(model, prompt).cloned())
}

pub fn usd(usage: &Usage, price: &ModelPrice) -> Option<f64> {
    let writes = usage.cache_writes.iter().try_fold(0.0, |sum, write| {
        price
            .cache_writes
            .iter()
            .find(|tier| tier.tier == write.tier)
            .map(|tier| sum + write.tokens as f64 * tier.usd_per_mtok)
    })?;
    Some(
        (usage.input_uncached as f64 * price.input
            + usage.cache_read as f64 * price.cache_read
            + writes
            + usage.output as f64 * price.output)
            / 1_000_000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::usage::{CacheWrite, TierRate};

    fn price(input: f64) -> ModelPrice {
        ModelPrice {
            input,
            cache_read: input / 10.0,
            cache_writes: vec![
                TierPrice {
                    tier: FIVE_MIN,
                    usd_per_mtok: input * 1.25,
                },
                TierPrice {
                    tier: ONE_HOUR,
                    usd_per_mtok: input * 2.0,
                },
            ],
            output: input * 5.0,
        }
    }

    fn entry(model: &str, above: Option<u64>, input: f64) -> PriceEntry {
        PriceEntry {
            model: model.into(),
            above_prompt_tokens: above,
            price: price(input),
        }
    }

    #[test]
    fn long_prompt_is_true_only_above_the_tier_line() {
        let table = table();
        assert!(!long_prompt(&table, "claude-haiku-5-5", 100_000));
        assert!(long_prompt(&table, "claude-haiku-5-5", 100_001));
        assert!(!long_prompt(&table, "claude-opus-5", 1_000_000));
        assert!(!long_prompt(&table, "unlisted", 1_000_000));
    }

    fn table() -> PriceTable {
        PriceTable {
            as_of: "2026-10-21",
            entries: vec![
                entry("claude-haiku-5", None, 0.1),
                entry("claude-haiku-5", Some(100_000), 0.5),
                entry("claude-haiku-5-5", None, 1.0),
                entry("claude-haiku-5-5", Some(100_000), 5.0),
            ],
        }
    }

    fn rates() -> BillingRates {
        BillingRates {
            cache_read: 0.1,
            cache_writes: vec![
                TierRate {
                    tier: FIVE_MIN,
                    multiplier: 1.25,
                },
                TierRate {
                    tier: ONE_HOUR,
                    multiplier: 2.0,
                },
            ],
            output: 5.0,
        }
    }

    fn usage(input: u64, read: u64, writes: Vec<CacheWrite>, output: u64) -> Usage {
        Usage {
            input_uncached: input,
            cache_read: read,
            cache_writes: writes,
            output,
            reasoning: None,
            other: vec![],
        }
    }

    fn overrides(input: f64) -> BTreeMap<String, PriceOverride> {
        BTreeMap::from([(
            "claude-haiku-5-5".to_string(),
            PriceOverride {
                input,
                cache_read: None,
                cache_write_5m: None,
                cache_write_1h: None,
                output: None,
            },
        )])
    }

    #[test]
    fn usd_prices_each_category() {
        let writes = vec![
            CacheWrite {
                tier: FIVE_MIN,
                tokens: 400_000,
            },
            CacheWrite {
                tier: ONE_HOUR,
                tokens: 100_000,
            },
        ];
        let u = usage(1_000_000, 2_000_000, writes, 500_000);
        let expected = 1.0 + 2.0 * 0.1 + 0.4 * 1.25 + 0.1 * 2.0 + 0.5 * 5.0;
        let got = usd(&u, &price(1.0)).unwrap();
        assert!((got - expected).abs() < 1e-9, "{got} vs {expected}");
    }

    #[test]
    fn unspecified_write_tier_is_unpriceable() {
        let writes = vec![CacheWrite {
            tier: CacheTier::Unspecified,
            tokens: 1,
        }];
        assert_eq!(usd(&usage(1, 0, writes, 1), &price(1.0)), None);
    }

    #[test]
    fn haiku_boundary_is_exclusive_and_counts_cache() {
        let t = table();
        assert_eq!(t.lookup("claude-haiku-5-5", 100_000).unwrap().input, 1.0);
        assert_eq!(t.lookup("claude-haiku-5-5", 100_001).unwrap().input, 5.0);
        let u = usage(
            40_000,
            30_000,
            vec![CacheWrite {
                tier: FIVE_MIN,
                tokens: 30_001,
            }],
            0,
        );
        assert_eq!(prompt_tokens(&u), 100_001);
    }

    #[test]
    fn lookup_prefers_longest_prefix_strips_suffix_and_respects_boundary() {
        let t = table();
        assert_eq!(t.lookup("claude-haiku-5-5-20260101", 1).unwrap().input, 1.0);
        assert_eq!(t.lookup("claude-haiku-5-5[1m]", 1).unwrap().input, 1.0);
        assert!(t.lookup("claude-haiku-5-6", 1).is_none());
        assert!(t.lookup("claude-haiku-5-5-latest", 1).is_none());
        assert!(t.lookup("claude-haiku-50", 1).is_none());
        assert!(t.lookup("gpt-4", 1).is_none());
    }

    #[test]
    fn override_full_wins_for_every_prompt_size() {
        let mut over = overrides(2.0);
        let entry = over.get_mut("claude-haiku-5-5").unwrap();
        entry.cache_read = Some(0.3);
        entry.cache_write_5m = Some(3.0);
        entry.cache_write_1h = Some(4.0);
        entry.output = Some(9.0);
        for prompt in [1, 200_000] {
            let got = resolve_price(&table(), &over, &rates(), "claude-haiku-5-5", prompt).unwrap();
            assert_eq!(got.input, 2.0);
            assert_eq!(got.cache_read, 0.3);
            assert_eq!(got.cache_writes[0].usd_per_mtok, 3.0);
            assert_eq!(got.cache_writes[1].usd_per_mtok, 4.0);
            assert_eq!(got.output, 9.0);
        }
    }

    #[test]
    fn override_partial_derives_from_provider_multipliers() {
        let got =
            resolve_price(&table(), &overrides(2.0), &rates(), "claude-haiku-5-5", 1).unwrap();
        assert_eq!(got.cache_read, 0.2);
        assert_eq!(got.cache_writes[0].usd_per_mtok, 2.5);
        assert_eq!(got.cache_writes[1].usd_per_mtok, 4.0);
        assert_eq!(got.output, 10.0);
    }

    #[test]
    fn no_override_uses_table_and_unknown_model_is_none() {
        let none = BTreeMap::new();
        let got = resolve_price(&table(), &none, &rates(), "claude-haiku-5-5", 100_001).unwrap();
        assert_eq!(got.input, 5.0);
        assert!(resolve_price(&table(), &none, &rates(), "mystery", 1).is_none());
        let other = overrides(2.0);
        assert!(resolve_price(&table(), &other, &rates(), "claude-haiku-5-6", 1).is_none());
    }

    #[test]
    fn override_applies_to_unknown_model() {
        let mut over = overrides(2.0);
        over.insert(
            "mystery".to_string(),
            PriceOverride {
                input: 7.0,
                cache_read: None,
                cache_write_5m: None,
                cache_write_1h: None,
                output: None,
            },
        );
        assert!(resolve_price(&table(), &over, &rates(), "mystery-1", 1).is_none());
        let got = resolve_price(&table(), &over, &rates(), "mystery", 1).unwrap();
        assert_eq!(got.input, 7.0);
        let dated = resolve_price(&table(), &over, &rates(), "mystery-20261021", 1).unwrap();
        assert_eq!(dated.input, 7.0);
    }

    #[test]
    fn negative_override_price_is_rejected() {
        let mut over = overrides(1.0);
        assert!(validate_price_overrides(&over).is_ok());
        over.get_mut("claude-haiku-5-5").unwrap().output = Some(-0.5);
        let error = validate_price_overrides(&over).unwrap_err();
        assert!(error.contains("claude-haiku-5-5"));
        assert!(error.contains("output"));
        over.get_mut("claude-haiku-5-5").unwrap().output = None;
        over.get_mut("claude-haiku-5-5").unwrap().input = -1.0;
        assert!(validate_price_overrides(&over)
            .unwrap_err()
            .contains("input"));
    }
}
