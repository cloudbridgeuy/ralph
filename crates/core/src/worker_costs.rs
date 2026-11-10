pub mod args;
pub mod tickets;
pub mod usage_report;
pub mod views;
pub mod window;

use crate::chat_memory::MAIN_HARNESS;
use crate::chat_process::UsageLine;
use crate::wire::price::{long_prompt, prompt_tokens, resolve_price, usd, PriceTable};
use crate::wire::provider::{provider, Provider, ProviderId};
use crate::wire::usage::{CacheWrite, Usage, FIVE_MIN, ONE_HOUR};
use crate::wire::view::Pricing;
use std::sync::LazyLock;

use crate::wire::provider::openai::price as openai_price;

use crate::wire::usage::THIRTY_MIN;

pub const NO_USAGE: &str = "No usage recorded.";
const UNKNOWN: &str = "unknown";
const MISSING: &str = "–";
const NO_PRICE: &str = "n/a";
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Main,
    Nap,
    Ask,
    Subagent,
    Worker(String),
}

impl Source {
    fn of(line: &UsageLine) -> Option<Source> {
        match line.kind.as_str() {
            "turn" | "ping" => Some(Source::Main),
            "nap" | "merge" | "tag" => Some(Source::Nap),
            "ask" => Some(Source::Ask),
            "subagent" => Some(Source::Subagent),
            "worker" => Some(Source::Worker(
                line.harness.clone().unwrap_or_else(|| UNKNOWN.to_string()),
            )),
            _ => None,
        }
    }

    fn label(&self) -> &str {
        match self {
            Source::Main => "main",
            Source::Nap => "nap",
            Source::Ask => "ask",
            Source::Subagent => "subagent",
            Source::Worker(harness) => harness,
        }
    }
}

fn either_sum(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    a.zip(b).map(|(a, b)| a.saturating_add(b)).or(a).or(b)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    write_5m: Option<u64>,
    write_1h: Option<u64>,
    partial_5m: bool,
    partial_1h: bool,
    partial_reasoning: bool,
    reasoning: Option<u64>,
}

impl Tokens {
    fn of(line: &UsageLine) -> Tokens {
        Tokens {
            input: line.input_tokens,
            output: line.output_tokens,
            cache_read: line.cache_read_input_tokens,
            cache_write: line.cache_creation_input_tokens,
            write_5m: line.cache_write_5m,
            write_1h: line.cache_write_1h,
            partial_5m: line.cache_write_5m.is_none(),
            partial_1h: line.cache_write_1h.is_none(),
            partial_reasoning: line.reasoning_tokens.is_none(),
            reasoning: line.reasoning_tokens,
        }
    }

    fn plus(self, other: Tokens) -> Tokens {
        Tokens {
            input: self.input.saturating_add(other.input),
            output: self.output.saturating_add(other.output),
            cache_read: self.cache_read.saturating_add(other.cache_read),
            cache_write: self.cache_write.saturating_add(other.cache_write),
            write_5m: either_sum(self.write_5m, other.write_5m),
            write_1h: either_sum(self.write_1h, other.write_1h),
            partial_5m: self.partial_5m || other.partial_5m,
            partial_1h: self.partial_1h || other.partial_1h,
            partial_reasoning: self.partial_reasoning || other.partial_reasoning,
            reasoning: either_sum(self.reasoning, other.reasoning),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usd {
    known: Option<f64>,
    missing: bool,
    estimate: bool,
    reported: bool,
}

impl Usd {
    fn of(cost: Option<f64>) -> Usd {
        Usd {
            known: cost,
            missing: cost.is_none(),
            estimate: false,
            reported: false,
        }
    }

    fn reported(cost: f64) -> Usd {
        Usd {
            known: Some(cost),
            missing: false,
            estimate: false,
            reported: true,
        }
    }

    fn plus(self, other: Usd) -> Usd {
        Usd {
            known: self
                .known
                .zip(other.known)
                .map(|(a, b)| a + b)
                .or(self.known)
                .or(other.known),
            missing: self.missing || other.missing,
            estimate: self.estimate || other.estimate,
            reported: self.reported || other.reported,
        }
    }

    fn text(self) -> String {
        let mark = if self.estimate { "~" } else { "" };
        let source = if self.reported && !self.estimate {
            " reported"
        } else {
            ""
        };
        match (self.known, self.missing) {
            (None, _) => NO_PRICE.to_string(),
            (Some(usd), true) => format!("{mark}{usd:.2}+{source}"),
            (Some(usd), false) => format!("{mark}{usd:.2}{source}"),
        }
    }
}

fn untiered_writes(line: &UsageLine) -> u64 {
    line.cache_creation_input_tokens.saturating_sub(
        line.cache_write_5m
            .unwrap_or(0)
            .saturating_add(line.cache_write_1h.unwrap_or(0)),
    )
}

fn tiered_usage(line: &UsageLine) -> Usage {
    let writes = [
        (FIVE_MIN, line.cache_write_5m.unwrap_or(0)),
        (ONE_HOUR, line.cache_write_1h.unwrap_or(0)),
    ];
    Usage {
        input_uncached: line.input_tokens,
        cache_read: line.cache_read_input_tokens,
        cache_writes: writes
            .into_iter()
            .filter(|(_, tokens)| *tokens > 0)
            .map(|(tier, tokens)| CacheWrite { tier, tokens })
            .collect(),
        output: line.output_tokens,
        reasoning: line.reasoning_tokens,
        other: vec![],
    }
}

pub fn builtin_prices(provider: &dyn Provider) -> PriceTable {
    let mut table = provider.prices();
    let missing: Vec<_> = openai_price::entries()
        .into_iter()
        .filter(|entry| !table.entries.iter().any(|have| have.model == entry.model))
        .collect();
    table.entries.extend(missing);
    table
}

static ANTHROPIC_PRICES: LazyLock<PriceTable> =
    LazyLock::new(|| builtin_prices(provider(ProviderId::Anthropic).as_ref()));

pub fn long_prompt_call(model: &str, usage: &Usage) -> bool {
    long_prompt(&ANTHROPIC_PRICES, model, prompt_tokens(usage))
}

fn tier_prompt(long: Option<bool>, summed: u64) -> u64 {
    match long {
        Some(true) => u64::MAX,
        Some(false) => 0,
        None => summed,
    }
}

fn computed_usd(line: &UsageLine, pricing: &Pricing, table: &PriceTable) -> Usd {
    let untiered = untiered_writes(line);
    let priced = line.model.as_deref().and_then(|model| {
        let mut usage = tiered_usage(line);
        let price = resolve_price(
            table,
            pricing.overrides,
            &pricing.provider.rates(model),
            model,
            tier_prompt(
                line.long_prompt,
                prompt_tokens(&usage).saturating_add(untiered),
            ),
        )?;
        let assumed = if line.harness.as_deref() == Some(MAIN_HARNESS) {
            ONE_HOUR
        } else {
            THIRTY_MIN
        };
        let priced_tier = price.cache_writes.iter().any(|write| write.tier == assumed);
        if priced_tier && untiered > 0 {
            usage.cache_writes.push(CacheWrite {
                tier: assumed,
                tokens: untiered,
            });
        }
        usd(&usage, &price).map(|usd| (usd, priced_tier))
    });
    Usd {
        known: priced.map(|(usd, _)| usd),
        missing: priced.is_none_or(|(_, priced_tier)| untiered > 0 && !priced_tier),
        estimate: true,
        reported: false,
    }
}

fn summed(lines: &[UsageLine], priced: &dyn Fn(&UsageLine) -> Usd) -> Usd {
    lines.iter().map(priced).fold(Usd::default(), Usd::plus)
}

fn count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{}.{}k", n / 1_000, n % 1_000 / 100),
        1_000_000..=999_999_999 => format!("{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000),
        _ => format!("{}.{}B", n / 1_000_000_000, n % 1_000_000_000 / 100_000_000),
    }
}

fn maybe(n: Option<u64>) -> String {
    n.map_or(MISSING.to_string(), count)
}

fn tier(known: Option<u64>, partial: bool) -> String {
    match (known, partial) {
        (Some(n), true) => format!("{}+", count(n)),
        (known, _) => maybe(known),
    }
}

#[cfg(test)]
mod tests;
