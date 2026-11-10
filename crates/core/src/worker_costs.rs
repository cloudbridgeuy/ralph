pub mod args;
pub mod tickets;
pub mod usage_report;
pub mod views;
pub mod window;

use crate::chat_memory::MAIN_HARNESS;
use crate::chat_process::{processes, session_cost, UsageLine};
use crate::wire::price::{long_prompt, prompt_tokens, resolve_price, usd, PriceTable};
use crate::wire::provider::{provider, Provider, ProviderId};
use crate::wire::usage::{CacheWrite, Usage, FIVE_MIN, ONE_HOUR};
use crate::wire::view::Pricing;
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;
use window::{in_window, instant};

use crate::wire::provider::openai::price as openai_price;

use crate::wire::usage::THIRTY_MIN;

pub const NO_USAGE: &str = "No usage recorded.";
const UNKNOWN: &str = "unknown";
const MISSING: &str = "–";
const NO_PRICE: &str = "n/a";
const HEADERS: [&str; 12] = [
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
    "HARNESS USD",
];

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

#[derive(Debug, Clone, PartialEq)]
struct CostRow {
    harness: String,
    model: String,
    runs: Option<usize>,
    tokens: Tokens,
    usd: Usd,
    harness_usd: Usd,
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

struct MainProcess {
    run: Option<String>,
    rows: Vec<UsageLine>,
    baseline: f64,
}

fn baseline(rows: &[UsageLine], since: Option<DateTime<Utc>>) -> f64 {
    since
        .and_then(|since| {
            rows.iter()
                .rev()
                .find_map(|row| instant(&row.at).filter(|at| *at < since).and(row.cost_usd))
        })
        .unwrap_or(0.0)
}

fn main_runs(processes: &[MainProcess]) -> (usize, Usd) {
    let runs: BTreeSet<_> = processes
        .iter()
        .map(|process| process.run.as_deref())
        .collect();
    let usd = processes
        .iter()
        .map(|process| {
            session_cost(&process.rows).map_or_else(
                || Usd::of(None),
                |cost| Usd {
                    known: Some((cost.total - process.baseline).max(0.0)),
                    missing: cost.missing,
                    estimate: false,
                    reported: false,
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
    main: &[MainProcess],
    priced: &dyn Fn(&UsageLine) -> Usd,
) -> CostRow {
    let harness = |line: &UsageLine| Usd::of(line.cost_usd);
    let (runs, usd, harness_usd) = match source {
        Source::Main => {
            let (runs, harness_usd) = main_runs(main);
            (runs, summed(lines, priced), harness_usd)
        }
        Source::Nap | Source::Ask | Source::Subagent => {
            (lines.len(), summed(lines, priced), summed(lines, &harness))
        }
        Source::Worker(_) => (
            worker_runs(lines),
            summed(lines, priced),
            summed(lines, &harness),
        ),
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
        harness_usd,
    }
}

fn main_processes(
    lines: Vec<UsageLine>,
    since: Option<DateTime<Utc>>,
) -> Vec<(String, MainProcess)> {
    let mut runs: BTreeMap<Option<String>, Vec<UsageLine>> = BTreeMap::new();
    for line in lines {
        runs.entry(line.run.clone()).or_default().push(line);
    }
    runs.iter()
        .flat_map(|(run, rows)| {
            processes(rows).into_iter().filter_map(|process| {
                let within: Vec<UsageLine> = process
                    .iter()
                    .filter(|row| in_window(&row.at, since))
                    .cloned()
                    .collect();
                if within.is_empty() {
                    return None;
                }
                let model = within
                    .iter()
                    .rev()
                    .find_map(|row| row.model.clone())
                    .unwrap_or_else(|| UNKNOWN.to_string());
                Some((
                    model,
                    MainProcess {
                        run: run.clone(),
                        baseline: baseline(process, since),
                        rows: within,
                    },
                ))
            })
        })
        .collect()
}

fn cost_rows(usage_jsonl: &str, since: Option<DateTime<Utc>>, pricing: &Pricing) -> Vec<CostRow> {
    let table = builtin_prices(pricing.provider);
    let priced = |line: &UsageLine| computed_usd(line, pricing, &table);
    let mut groups: BTreeMap<(Source, String), Vec<UsageLine>> = BTreeMap::new();
    let mut main = Vec::new();
    for line in usage_jsonl
        .lines()
        .filter_map(|line| serde_json::from_str::<UsageLine>(line).ok())
        .filter(|line| in_window(&line.at, since) || Source::of(line) == Some(Source::Main))
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
    let mut processes: BTreeMap<String, Vec<MainProcess>> = BTreeMap::new();
    for (model, process) in main_processes(main, since) {
        groups
            .entry((Source::Main, model.clone()))
            .or_default()
            .extend(process.rows.iter().cloned());
        processes.entry(model).or_default().push(process);
    }
    groups
        .into_iter()
        .map(|((source, model), lines)| {
            let own = match source {
                Source::Main => processes.get(&model).map_or(&[][..], Vec::as_slice),
                _ => &[],
            };
            cost_row(&source, model, &lines, own, &priced)
        })
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
        harness_usd: rows
            .iter()
            .map(|row| row.harness_usd)
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

fn tier(known: Option<u64>, partial: bool) -> String {
    match (known, partial) {
        (Some(n), true) => format!("{}+", count(n)),
        (known, _) => maybe(known),
    }
}

fn cells(row: &CostRow) -> [String; 12] {
    let tokens = row.tokens;
    [
        row.harness.clone(),
        row.model.clone(),
        row.runs.map_or(String::new(), |runs| runs.to_string()),
        count(tokens.input),
        count(tokens.output),
        count(tokens.cache_read),
        count(tokens.cache_write),
        tier(tokens.write_5m, tokens.partial_5m),
        tier(tokens.write_1h, tokens.partial_1h),
        tier(tokens.reasoning, tokens.partial_reasoning),
        row.usd.text(),
        row.harness_usd.text(),
    ]
}

pub fn costs_view(usage_jsonl: &str, since: Option<DateTime<Utc>>, pricing: &Pricing) -> String {
    let mut rows = cost_rows(usage_jsonl, since, pricing);
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
