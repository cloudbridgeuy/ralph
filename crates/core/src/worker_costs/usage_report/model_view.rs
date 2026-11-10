use super::{label_excluded, parse, partition, Entry};
use crate::chat_process::{processes, session_cost, UsageLine};
use crate::wire::view::Pricing;
use crate::worker_costs::window::{in_window, instant};
use crate::worker_costs::{
    builtin_prices, computed_usd, count, summed, tier, Source, Tokens, Usd, NO_USAGE, UNKNOWN,
};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, BTreeSet};

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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CostRow {
    pub(crate) harness: String,
    pub(crate) model: String,
    pub(crate) runs: Option<usize>,
    pub(crate) tokens: Tokens,
    pub(crate) usd: Usd,
    pub(crate) harness_usd: Usd,
    pub(crate) metered: usize,
    pub(crate) excluded: usize,
}

struct MainProcess {
    run: Option<String>,
    rows: Vec<UsageLine>,
    baseline: f64,
}

pub(crate) fn baseline(rows: &[UsageLine], since: Option<DateTime<Utc>>) -> f64 {
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
        metered: lines.len(),
        excluded: lines.iter().filter(|line| super::is_excluded(line)).count(),
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

pub(crate) fn cost_rows(
    usage_jsonl: &str,
    since: Option<DateTime<Utc>>,
    pricing: &Pricing,
) -> Vec<CostRow> {
    let table = builtin_prices(pricing.provider);
    let priced = |line: &UsageLine| computed_usd(line, pricing, &table);
    let ledger = parse(usage_jsonl, &[]);
    let (main, others): (Vec<&Entry>, Vec<&Entry>) = ledger
        .entries
        .iter()
        .filter(|entry| entry.source == Source::Main || since.is_none_or(|since| entry.at >= since))
        .partition(|entry| entry.source == Source::Main);
    let mut groups = partition(others.into_iter(), |entry| {
        (
            entry.source.clone(),
            entry
                .line
                .model
                .clone()
                .unwrap_or_else(|| UNKNOWN.to_string()),
        )
    })
    .into_iter()
    .map(|(key, entries)| {
        (
            key,
            entries
                .into_iter()
                .map(|entry| entry.line.clone())
                .collect::<Vec<_>>(),
        )
    })
    .collect::<BTreeMap<_, _>>();
    let mut processes: BTreeMap<String, Vec<MainProcess>> = BTreeMap::new();
    for (model, process) in
        main_processes(main.iter().map(|entry| entry.line.clone()).collect(), since)
    {
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

pub(crate) fn total_row(rows: &[CostRow]) -> CostRow {
    let counted = rows.iter().filter(|row| row.excluded < row.metered);
    CostRow {
        harness: "total".to_string(),
        model: String::new(),
        runs: None,
        tokens: rows
            .iter()
            .map(|row| row.tokens)
            .fold(Tokens::default(), Tokens::plus),
        usd: counted
            .clone()
            .map(|row| row.usd)
            .fold(Usd::default(), Usd::plus),
        harness_usd: counted
            .map(|row| row.harness_usd)
            .fold(Usd::default(), Usd::plus),
        metered: rows.iter().map(|row| row.metered).sum(),
        excluded: rows.iter().map(|row| row.excluded).sum(),
    }
}

pub(crate) fn cells(row: &CostRow) -> [String; 12] {
    let tokens = row.tokens;
    let money = |usd: Usd| label_excluded(usd.text(), row.excluded, row.metered);
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
        money(row.usd),
        money(row.harness_usd),
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
