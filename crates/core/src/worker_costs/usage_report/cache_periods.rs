use super::cache_view::{ProcessStart, START_ORDER};
use super::{Entry, Role};
use crate::chat_process::{restart_write, UsageLine, CACHE_TTL_SECS};
use crate::worker_costs::window::instant;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeriodOutcome {
    Warm,
    Cold,
    NoNextTurn,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IdlePeriod {
    pub run: Option<String>,
    pub pings: u64,
    pub ping_cost: f64,
    pub last_turn_at: Option<DateTime<Utc>>,
    pub last_ping_at: Option<DateTime<Utc>>,
    pub context_tokens: Option<u64>,
    pub model: Option<String>,
    pub next_turn_at: Option<DateTime<Utc>>,
    pub outcome: PeriodOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeCounts {
    pub periods: usize,
    pub pings: u64,
    pub ping_cost: f64,
    pub warm: usize,
    pub cold: usize,
    pub no_next: usize,
}

struct Row<'a> {
    line: &'a UsageLine,
    at: DateTime<Utc>,
}

impl Row<'_> {
    fn happened(&self) -> DateTime<Utc> {
        self.line
            .request_at
            .as_deref()
            .and_then(instant)
            .unwrap_or(self.at)
    }
}

fn main_runs(entries: &[Entry]) -> BTreeMap<Option<String>, Vec<Row<'_>>> {
    let mut runs: BTreeMap<Option<String>, Vec<Row>> = BTreeMap::new();
    for entry in entries.iter().filter(|entry| entry.role == Role::Main) {
        runs.entry(entry.line.run.clone()).or_default().push(Row {
            line: &entry.line,
            at: entry.at,
        });
    }
    runs
}

pub fn idle_periods(entries: &[Entry]) -> Vec<IdlePeriod> {
    main_runs(entries)
        .iter()
        .flat_map(|(run, rows)| run_periods(run, rows))
        .collect()
}

fn run_periods(run: &Option<String>, rows: &[Row]) -> Vec<IdlePeriod> {
    let mut periods = Vec::new();
    let mut last_turn: Option<&Row> = None;
    let mut pings: Vec<&Row> = Vec::new();
    for row in rows {
        match row.line.kind.as_str() {
            "turn" => {
                if let Some(turn) = last_turn {
                    if !pings.is_empty() {
                        periods.push(period(run, turn, &pings, Some(row)));
                    }
                }
                pings.clear();
                last_turn = Some(row);
            }
            "ping" if last_turn.is_some() => pings.push(row),
            _ => {}
        }
    }
    if let Some(turn) = last_turn {
        if !pings.is_empty() {
            periods.push(period(run, turn, &pings, None));
        }
    }
    periods
}

fn period(run: &Option<String>, turn: &Row, pings: &[&Row], next: Option<&Row>) -> IdlePeriod {
    IdlePeriod {
        run: run.clone(),
        pings: pings.len() as u64,
        ping_cost: pings
            .iter()
            .map(|ping| ping.line.cost_usd.unwrap_or(0.0))
            .sum(),
        last_turn_at: Some(turn.happened()),
        last_ping_at: pings.last().map(|ping| ping.happened()),
        context_tokens: turn.line.context_tokens,
        model: turn.line.model.clone(),
        next_turn_at: next.map(Row::happened),
        outcome: next.map_or(PeriodOutcome::NoNextTurn, |next| outcome_of(next.line)),
    }
}

fn outcome_of(next: &UsageLine) -> PeriodOutcome {
    let cold = next.process_turn == Some(1)
        || restart_write(
            next.input_tokens,
            next.cache_creation_input_tokens,
            next.cache_read_input_tokens,
        );
    if cold {
        PeriodOutcome::Cold
    } else {
        PeriodOutcome::Warm
    }
}

pub fn outcome_counts(periods: &[IdlePeriod]) -> OutcomeCounts {
    let count = |outcome| {
        periods
            .iter()
            .filter(|period| period.outcome == outcome)
            .count()
    };
    OutcomeCounts {
        periods: periods.len(),
        pings: periods.iter().map(|period| period.pings).sum(),
        ping_cost: periods.iter().map(|period| period.ping_cost).sum(),
        warm: count(PeriodOutcome::Warm),
        cold: count(PeriodOutcome::Cold),
        no_next: count(PeriodOutcome::NoNextTurn),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResetWrites {
    pub start: ProcessStart,
    pub turns: usize,
    pub mean_ratio: Option<f64>,
    pub mean_write_tokens: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MissCauses {
    pub misses: usize,
    pub reset: usize,
    pub ttl: usize,
    pub unexplained: usize,
    pub new_run: usize,
    pub turns: usize,
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn start_of(line: &UsageLine) -> ProcessStart {
    line.reset.map_or(ProcessStart::NewRun, ProcessStart::Reset)
}

pub fn reset_writes(entries: &[Entry]) -> Vec<ResetWrites> {
    let firsts: Vec<&UsageLine> = main_runs(entries)
        .into_values()
        .flatten()
        .map(|row| row.line)
        .filter(|line| line.kind == "turn" && line.process_turn == Some(1))
        .collect();
    START_ORDER
        .iter()
        .filter_map(|(_, start)| {
            let group: Vec<&UsageLine> = firsts
                .iter()
                .copied()
                .filter(|line| start_of(line) == *start)
                .collect();
            let ratios: Vec<f64> = group
                .iter()
                .filter_map(|line| {
                    let context = line.context_tokens.filter(|context| *context > 0)?;
                    Some(line.cache_creation_input_tokens as f64 / context as f64)
                })
                .collect();
            let writes: Vec<f64> = group
                .iter()
                .map(|line| line.cache_creation_input_tokens as f64)
                .collect();
            (!group.is_empty()).then(|| ResetWrites {
                start: *start,
                turns: group.len(),
                mean_ratio: mean(&ratios),
                mean_write_tokens: mean(&writes),
            })
        })
        .collect()
}

pub fn miss_causes(entries: &[Entry]) -> MissCauses {
    let mut causes = MissCauses::default();
    for rows in main_runs(entries).values() {
        let mut previous: Option<DateTime<Utc>> = None;
        for row in rows {
            let line = row.line;
            if line.kind != "turn" && line.kind != "ping" {
                continue;
            }
            let now = row.happened();
            if line.kind == "turn" {
                causes.turns += 1;
                let after_reset = line.process_turn == Some(1) && line.reset.is_some();
                let write_heavy = restart_write(
                    line.input_tokens,
                    line.cache_creation_input_tokens,
                    line.cache_read_input_tokens,
                );
                if after_reset || write_heavy {
                    causes.misses += 1;
                    match (line.process_turn, line.reset) {
                        (Some(1), Some(_)) => causes.reset += 1,
                        (Some(1), None) => causes.new_run += 1,
                        _ if previous
                            .is_some_and(|prev| (now - prev).num_seconds() > CACHE_TTL_SECS) =>
                        {
                            causes.ttl += 1
                        }
                        _ => causes.unexplained += 1,
                    }
                }
            }
            previous = Some(now);
        }
    }
    causes
}

#[cfg(test)]
#[path = "cache_periods_tests.rs"]
mod cache_periods_tests;
