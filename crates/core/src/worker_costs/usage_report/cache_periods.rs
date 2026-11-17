use super::{Entry, Role};
use crate::chat_process::{restart_write, UsageLine};
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

pub fn idle_periods(entries: &[Entry]) -> Vec<IdlePeriod> {
    let mut runs: BTreeMap<Option<String>, Vec<Row>> = BTreeMap::new();
    for entry in entries.iter().filter(|entry| entry.role == Role::Main) {
        runs.entry(entry.line.run.clone()).or_default().push(Row {
            line: &entry.line,
            at: entry.at,
        });
    }
    runs.iter()
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

#[cfg(test)]
#[path = "cache_periods_tests.rs"]
mod cache_periods_tests;
