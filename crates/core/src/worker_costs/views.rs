use super::tickets::{parse_windows, window_position, WorkerWindow, UNKNOWN_TICKET};
use super::window::{in_window, instant};
use super::{
    builtin_prices, computed_usd, costs_view, tier, Source, Tokens, Usd, MISSING, NO_USAGE, UNKNOWN,
};
use crate::chat_process::UsageLine;
use crate::wire::view::Pricing;
use crate::worker::HarnessId;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};

const HARNESS_SOURCE: &str = "harness";
const HEADERS: [&str; 8] = [
    "INPUT",
    "OUTPUT",
    "CACHE READ",
    "CACHE WRITE",
    "5M",
    "1H",
    "REASONING",
    "USD",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    HarnessModel,
    Harness,
    Role,
    Ticket,
}

#[derive(Clone, Copy)]
enum Grouping {
    Harness,
    Role,
    Ticket,
}

pub struct Inputs<'a> {
    pub usage: &'a str,
    pub workers: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Main,
    Nap,
    Merge,
    Ask,
    Subagent,
    Worker,
}

impl Role {
    fn of(kind: &str) -> Option<Role> {
        match kind {
            "turn" | "ping" => Some(Role::Main),
            "nap" => Some(Role::Nap),
            "merge" => Some(Role::Merge),
            "ask" => Some(Role::Ask),
            "subagent" => Some(Role::Subagent),
            "worker" => Some(Role::Worker),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Role::Main => "main",
            Role::Nap => "nap",
            Role::Merge => "merge",
            Role::Ask => "ask",
            Role::Subagent => "subagent",
            Role::Worker => "worker",
        }
    }
}

#[derive(Deserialize)]
struct CostSourceTag {
    cost_source: Option<String>,
}

struct Entry {
    line: UsageLine,
    window: Option<usize>,
    reported: bool,
}

struct Row {
    label: String,
    metered: usize,
    unmetered: usize,
    tokens: Tokens,
    usd: Usd,
}

impl Row {
    fn total(rows: &[Row]) -> Row {
        Row {
            label: "total".to_string(),
            metered: rows.iter().map(|row| row.metered).sum(),
            unmetered: rows.iter().map(|row| row.unmetered).sum(),
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

    fn cells(&self) -> Vec<String> {
        let mut cells = vec![self.label.clone()];
        if self.metered == 0 {
            cells.extend(HEADERS.map(|_| UNKNOWN.to_string()));
            return cells;
        }
        let partial = self.unmetered > 0;
        let mark = |text: String| {
            if partial && text != MISSING && !text.ends_with('+') {
                format!("{text}+")
            } else {
                text
            }
        };
        let tokens = self.tokens;
        let usd = Usd {
            missing: self.usd.missing || partial,
            ..self.usd
        };
        cells.extend(
            [
                super::count(tokens.input),
                super::count(tokens.output),
                super::count(tokens.cache_read),
                super::count(tokens.cache_write),
                tier(tokens.write_5m, tokens.partial_5m),
                tier(tokens.write_1h, tokens.partial_1h),
                tier(tokens.reasoning, tokens.partial_reasoning),
            ]
            .map(mark),
        );
        cells.push(usd.text());
        cells
    }
}

fn parse_lines(usage: &str, since: Option<DateTime<Utc>>) -> Vec<(UsageLine, bool)> {
    usage
        .lines()
        .filter_map(|text| {
            let line = serde_json::from_str::<UsageLine>(text).ok()?;
            Source::of(&line)?;
            let tag = serde_json::from_str::<CostSourceTag>(text).ok()?;
            let reported = tag.cost_source.as_deref() == Some(HARNESS_SOURCE)
                && line.cost_usd.is_some()
                && Role::of(&line.kind) != Some(Role::Main);
            in_window(&line.at, since).then_some((line, reported))
        })
        .collect()
}

fn entries(lines: Vec<(UsageLine, bool)>, windows: &[WorkerWindow]) -> Vec<Entry> {
    lines
        .into_iter()
        .map(|(line, reported)| {
            let window = (line.kind == "worker")
                .then(|| {
                    let id = line.worker_id.as_deref()?;
                    window_position(windows, id, instant(&line.at)?)
                })
                .flatten();
            Entry {
                line,
                window,
                reported,
            }
        })
        .collect()
}

fn unmetered_cursors<'a>(
    windows: &'a [WorkerWindow],
    entries: &[Entry],
    since: Option<DateTime<Utc>>,
) -> Vec<&'a WorkerWindow> {
    let joined: HashSet<usize> = entries.iter().filter_map(|entry| entry.window).collect();
    windows
        .iter()
        .enumerate()
        .filter(|(position, window)| {
            window.harness == HarnessId::Cursor.as_str()
                && !joined.contains(position)
                && since.is_none_or(|since| window.start >= since)
        })
        .map(|(_, window)| window)
        .collect()
}

fn build<K: Ord>(
    entries: &[Entry],
    unmetered: Vec<K>,
    key: impl Fn(&Entry) -> Option<K>,
    label: impl Fn(&K) -> String,
    priced: &dyn Fn(&Entry) -> Usd,
) -> Vec<Row> {
    let mut groups: BTreeMap<K, (Vec<&Entry>, usize)> = BTreeMap::new();
    for entry in entries {
        if let Some(key) = key(entry) {
            groups.entry(key).or_default().0.push(entry);
        }
    }
    for key in unmetered {
        groups.entry(key).or_default().1 += 1;
    }
    groups
        .into_iter()
        .map(|(key, (lines, unmetered))| Row {
            label: label(&key),
            metered: lines.len(),
            unmetered,
            tokens: lines
                .iter()
                .map(|entry| Tokens::of(&entry.line))
                .fold(Tokens::default(), Tokens::plus),
            usd: lines
                .iter()
                .map(|entry| priced(entry))
                .fold(Usd::default(), Usd::plus),
        })
        .collect()
}

fn ticket_key(ticket: Option<&str>) -> (bool, String) {
    match ticket {
        Some(ticket) => (false, ticket.to_string()),
        None => (true, UNKNOWN_TICKET.to_string()),
    }
}

fn rows(
    grouping: Grouping,
    entries: &[Entry],
    windows: &[WorkerWindow],
    since: Option<DateTime<Utc>>,
    priced: &dyn Fn(&Entry) -> Usd,
) -> Vec<Row> {
    let cursors = unmetered_cursors(windows, entries, since);
    match grouping {
        Grouping::Harness => build(
            entries,
            cursors
                .iter()
                .map(|window| Source::Worker(window.harness.clone()))
                .collect(),
            |entry| Source::of(&entry.line),
            |source| source.label().to_string(),
            priced,
        ),
        Grouping::Role => build(
            entries,
            cursors.iter().map(|_| Role::Worker).collect(),
            |entry| Role::of(&entry.line.kind),
            |role| role.label().to_string(),
            priced,
        ),
        Grouping::Ticket => build(
            entries,
            cursors
                .iter()
                .map(|window| ticket_key(window.ticket.as_deref()))
                .collect(),
            |entry| {
                Some(ticket_key(
                    entry
                        .window
                        .and_then(|position| windows[position].ticket.as_deref()),
                ))
            },
            |(_, label)| label.clone(),
            priced,
        ),
    }
}

fn key_header(grouping: Grouping) -> &'static str {
    match grouping {
        Grouping::Harness => "HARNESS",
        Grouping::Role => "ROLE",
        Grouping::Ticket => "TICKET",
    }
}

fn markdown(grouping: Grouping, rows: &[Row]) -> String {
    let line = |cells: &[String]| format!("| {} |", cells.join(" | "));
    let headers: Vec<String> = std::iter::once(key_header(grouping))
        .chain(HEADERS)
        .map(String::from)
        .collect();
    let mut lines = vec![
        line(&headers),
        line(&vec!["---".to_string(); headers.len()]),
    ];
    lines.extend(rows.iter().map(|row| line(&row.cells())));
    lines.push(line(&Row::total(rows).cells()));
    lines.join("\n")
}

pub fn costs_by_view(
    view: View,
    since: Option<DateTime<Utc>>,
    inputs: &Inputs,
    pricing: &Pricing,
) -> String {
    let grouping = match view {
        View::HarnessModel => return costs_view(inputs.usage, since, pricing),
        View::Harness => Grouping::Harness,
        View::Role => Grouping::Role,
        View::Ticket => Grouping::Ticket,
    };
    let windows = parse_windows(inputs.workers);
    let entries = entries(parse_lines(inputs.usage, since), &windows);
    let table = builtin_prices(pricing.provider);
    let priced = |entry: &Entry| match (entry.reported, entry.line.cost_usd) {
        (true, Some(cost)) => Usd::reported(cost),
        _ => computed_usd(&entry.line, pricing, &table),
    };
    let rows = rows(grouping, &entries, &windows, since, &priced);
    if rows.is_empty() {
        return NO_USAGE.to_string();
    }
    markdown(grouping, &rows)
}

#[cfg(test)]
mod tests;
