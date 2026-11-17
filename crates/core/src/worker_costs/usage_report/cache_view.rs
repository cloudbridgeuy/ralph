use super::{Entry, Role};
use crate::chat_process::{processes, ResetReason, UsageLine};
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStart {
    Reset(ResetReason),
    NewRun,
    Cut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessCost {
    pub run: Option<String>,
    pub cost: f64,
    pub turns: u64,
    pub pings: u64,
    pub start: ProcessStart,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessSummary {
    pub processes: usize,
    pub total: f64,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub median_turns: Option<u64>,
    pub by_start: Vec<(String, usize, f64)>,
}

pub fn runless_rows(entries: &[Entry]) -> usize {
    entries
        .iter()
        .filter(|entry| entry.role == Role::Main)
        .filter(|entry| matches!(entry.line.kind.as_str(), "turn" | "ping"))
        .filter(|entry| entry.line.run.is_none())
        .count()
}

pub fn process_costs(entries: &[Entry]) -> Vec<ProcessCost> {
    let mut runs: BTreeMap<Option<String>, Vec<UsageLine>> = BTreeMap::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.role == Role::Main && entry.line.run.is_some())
    {
        runs.entry(entry.line.run.clone())
            .or_default()
            .push(entry.line.clone());
    }
    runs.iter()
        .flat_map(|(run, lines)| {
            processes(lines)
                .into_iter()
                .map(|rows| process_cost(run, rows))
        })
        .collect()
}

fn process_cost(run: &Option<String>, rows: &[UsageLine]) -> ProcessCost {
    let start = match rows.first() {
        Some(first) if first.kind == "turn" && first.process_turn == Some(1) => first
            .reset
            .map_or(ProcessStart::NewRun, ProcessStart::Reset),
        _ => ProcessStart::Cut,
    };
    ProcessCost {
        run: run.clone(),
        cost: rows.iter().map(|row| row.cost_usd.unwrap_or(0.0)).sum(),
        turns: count_kind(rows, "turn"),
        pings: count_kind(rows, "ping"),
        start,
        complete: start != ProcessStart::Cut,
    }
}

fn count_kind(rows: &[UsageLine], kind: &str) -> u64 {
    rows.iter().filter(|row| row.kind == kind).count() as u64
}

fn sorted<T: PartialOrd + Copy>(values: impl Iterator<Item = T>) -> Vec<T> {
    let mut values: Vec<T> = values.collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    values
}

fn median(costs: &[f64]) -> Option<f64> {
    let middle = costs.len() / 2;
    match costs.len() {
        0 => None,
        len if len % 2 == 1 => costs.get(middle).copied(),
        _ => Some((costs.get(middle.checked_sub(1)?)? + costs.get(middle)?) / 2.0),
    }
}

fn median_lower(values: &[u64]) -> Option<u64> {
    values.get(values.len().checked_sub(1)? / 2).copied()
}

pub fn summarize(costs: &[ProcessCost]) -> ProcessSummary {
    let total: f64 = costs.iter().map(|cost| cost.cost).sum();
    let ordered = sorted(costs.iter().map(|cost| cost.cost));
    let complete: Vec<&ProcessCost> = costs.iter().filter(|cost| cost.complete).collect();
    let mut turns: Vec<u64> = complete.iter().map(|cost| cost.turns).collect();
    turns.sort_unstable();
    ProcessSummary {
        processes: costs.len(),
        total,
        mean: (!costs.is_empty()).then(|| total / costs.len() as f64),
        median: median(&ordered),
        median_turns: median_lower(&turns),
        by_start: START_ORDER
            .iter()
            .filter_map(|(label, start)| {
                let group: Vec<&&ProcessCost> = complete
                    .iter()
                    .filter(|cost| cost.start == *start)
                    .collect();
                (!group.is_empty()).then(|| {
                    (
                        label.to_string(),
                        group.len(),
                        group.iter().map(|cost| cost.cost).sum(),
                    )
                })
            })
            .collect(),
    }
}

pub(super) const START_ORDER: [(&str, ProcessStart); 4] = [
    ("tokens", ProcessStart::Reset(ResetReason::Tokens)),
    ("turns", ProcessStart::Reset(ResetReason::Turns)),
    ("restart", ProcessStart::Reset(ResetReason::Restart)),
    ("new run", ProcessStart::NewRun),
];

#[cfg(test)]
#[path = "cache_view_tests.rs"]
mod cache_view_tests;
