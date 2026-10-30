use crate::chat_thread::{processes, session_cost, UsageLine};
use crate::wire::price::{prompt_tokens, resolve_price, usd, PriceTable};
use crate::wire::usage::{CacheWrite, Usage, FIVE_MIN, ONE_HOUR};
use crate::wire::view::Pricing;
use std::collections::{BTreeMap, BTreeSet};

mod openai_price;

use openai_price::THIRTY_MIN;

pub const NO_USAGE: &str = "No usage recorded.";
const UNKNOWN: &str = "unknown";
const MISSING: &str = "–";
const NO_PRICE: &str = "n/a";
const HEADERS: [&str; 11] = [
    "HARNESS",
    "MODEL",
    "RUNS",
    "INPUT",
    "OUTPUT",
    "CACHE READ",
    "CACHE WRITE",
    "5M",
    "1H",
    "REASONING",
    "USD",
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    Main,
    Nap,
    Worker(String),
}

impl Source {
    fn of(line: &UsageLine) -> Option<Source> {
        match line.kind.as_str() {
            "turn" | "ping" => Some(Source::Main),
            "nap" => Some(Source::Nap),
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
            Source::Worker(harness) => harness,
        }
    }
}

fn either_sum(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    a.zip(b).map(|(a, b)| a.saturating_add(b)).or(a).or(b)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Tokens {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    write_5m: Option<u64>,
    write_1h: Option<u64>,
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
            reasoning: either_sum(self.reasoning, other.reasoning),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Usd {
    known: Option<f64>,
    missing: bool,
}

impl Usd {
    fn of(cost: Option<f64>) -> Usd {
        Usd {
            known: cost,
            missing: cost.is_none(),
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
        }
    }

    fn text(self) -> String {
        match (self.known, self.missing) {
            (None, _) => NO_PRICE.to_string(),
            (Some(usd), true) => format!("{usd:.2}+"),
            (Some(usd), false) => format!("{usd:.2}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CostRow {
    harness: String,
    model: String,
    runs: Option<usize>,
    tokens: Tokens,
    usd: Usd,
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

fn computed_usd(line: &UsageLine, pricing: &Pricing, table: &PriceTable) -> Usd {
    let untiered = untiered_writes(line);
    let priced = line.model.as_deref().and_then(|model| {
        let mut usage = tiered_usage(line);
        let price = resolve_price(
            table,
            pricing.overrides,
            &pricing.provider.rates(model),
            model,
            prompt_tokens(&usage).saturating_add(untiered),
        )?;
        let at_30m = price
            .cache_writes
            .iter()
            .any(|write| write.tier == THIRTY_MIN);
        if at_30m && untiered > 0 {
            usage.cache_writes.push(CacheWrite {
                tier: THIRTY_MIN,
                tokens: untiered,
            });
        }
        usd(&usage, &price).map(|usd| (usd, at_30m))
    });
    Usd {
        known: priced.map(|(usd, _)| usd),
        missing: priced.is_none_or(|(_, at_30m)| untiered > 0 && !at_30m),
    }
}

fn summed(lines: &[UsageLine], priced: &dyn Fn(&UsageLine) -> Usd) -> Usd {
    lines
        .iter()
        .map(|line| {
            line.cost_usd
                .map_or_else(|| priced(line), |cost| Usd::of(Some(cost)))
        })
        .fold(Usd::default(), Usd::plus)
}

fn main_runs(lines: &[UsageLine]) -> (usize, Usd) {
    let mut runs: BTreeMap<Option<&str>, Vec<UsageLine>> = BTreeMap::new();
    for line in lines {
        runs.entry(line.run.as_deref())
            .or_default()
            .push(line.clone());
    }
    let usd = runs
        .values()
        .map(|rows| {
            session_cost(rows).map_or_else(
                || Usd::of(None),
                |cost| Usd {
                    known: Some(cost.total),
                    missing: cost.missing,
                },
            )
        })
        .fold(Usd::default(), Usd::plus);
    (runs.len(), usd)
}

fn worker_runs(lines: &[UsageLine]) -> usize {
    lines
        .iter()
        .map(|line| (line.worker_id.as_deref(), line.session_id.as_deref()))
        .collect::<BTreeSet<_>>()
        .len()
}

fn cost_row(
    source: &Source,
    model: String,
    lines: &[UsageLine],
    priced: &dyn Fn(&UsageLine) -> Usd,
) -> CostRow {
    let (runs, usd) = match source {
        Source::Main => main_runs(lines),
        Source::Nap => (lines.len(), summed(lines, priced)),
        Source::Worker(_) => (worker_runs(lines), summed(lines, priced)),
    };
    CostRow {
        harness: source.label().to_string(),
        model,
        runs: Some(runs),
        tokens: lines
            .iter()
            .map(Tokens::of)
            .fold(Tokens::default(), Tokens::plus),
        usd,
    }
}

fn process_labeled(lines: Vec<UsageLine>) -> Vec<(String, UsageLine)> {
    let mut runs: BTreeMap<Option<String>, Vec<UsageLine>> = BTreeMap::new();
    for line in lines {
        runs.entry(line.run.clone()).or_default().push(line);
    }
    runs.values()
        .flat_map(|rows| processes(rows))
        .flat_map(|process| {
            let model = process
                .iter()
                .rev()
                .find_map(|row| row.model.clone())
                .unwrap_or_else(|| UNKNOWN.to_string());
            process.iter().map(move |row| (model.clone(), row.clone()))
        })
        .collect()
}

fn cost_rows(usage_jsonl: &str, pricing: &Pricing) -> Vec<CostRow> {
    let mut table = pricing.provider.prices();
    table.entries.extend(openai_price::entries());
    let priced = |line: &UsageLine| computed_usd(line, pricing, &table);
    let mut groups: BTreeMap<(Source, String), Vec<UsageLine>> = BTreeMap::new();
    let mut main = Vec::new();
    for line in usage_jsonl
        .lines()
        .filter_map(|line| serde_json::from_str::<UsageLine>(line).ok())
    {
        match Source::of(&line) {
            Some(Source::Main) => main.push(line),
            Some(source) => {
                let model = line.model.clone().unwrap_or_else(|| UNKNOWN.to_string());
                groups.entry((source, model)).or_default().push(line);
            }
            None => {}
        }
    }
    for (model, line) in process_labeled(main) {
        groups.entry((Source::Main, model)).or_default().push(line);
    }
    groups
        .into_iter()
        .map(|((source, model), lines)| cost_row(&source, model, &lines, &priced))
        .collect()
}

fn total_row(rows: &[CostRow]) -> CostRow {
    CostRow {
        harness: "total".to_string(),
        model: String::new(),
        runs: None,
        tokens: rows
            .iter()
            .map(|row| row.tokens)
            .fold(Tokens::default(), Tokens::plus),
        usd: rows
            .iter()
            .map(|row| row.usd)
            .fold(Usd::default(), Usd::plus),
    }
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

fn cells(row: &CostRow) -> [String; 11] {
    let tokens = row.tokens;
    [
        row.harness.clone(),
        row.model.clone(),
        row.runs.map_or(String::new(), |runs| runs.to_string()),
        count(tokens.input),
        count(tokens.output),
        count(tokens.cache_read),
        count(tokens.cache_write),
        maybe(tokens.write_5m),
        maybe(tokens.write_1h),
        maybe(tokens.reasoning),
        row.usd.text(),
    ]
}

pub fn costs_view(usage_jsonl: &str, pricing: &Pricing) -> String {
    let mut rows = cost_rows(usage_jsonl, pricing);
    if rows.is_empty() {
        return NO_USAGE.to_string();
    }
    rows.push(total_row(&rows));
    let line = |row: &[String]| format!("| {} |", row.join(" | "));
    let mut lines = vec![
        line(&HEADERS.map(String::from)),
        line(&vec!["---".to_string(); HEADERS.len()]),
    ];
    lines.extend(rows.iter().map(|row| line(&cells(row))));
    lines.join("\n")
}

#[cfg(test)]
mod tests;
