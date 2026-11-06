use super::tickets::{parse_windows, window_position, WorkerWindow, UNKNOWN_TICKET};
use super::window::instant;
use super::{
    builtin_prices, computed_usd, costs_view, tier, Source, Tokens, Usd, MISSING, NO_USAGE, UNKNOWN,
};
use crate::chat_process::UsageLine;
use crate::wire::view::Pricing;
use crate::worker::HarnessId;
use std::collections::{BTreeMap, HashSet};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
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

struct Entry {
    line: UsageLine,
    window: Option<usize>,
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
                super::maybe(tokens.reasoning),
            ]
            .map(mark),
        );
        cells.push(usd.text());
        cells
    }
}

fn parse_lines(usage: &str) -> Vec<UsageLine> {
    usage
        .lines()
        .filter_map(|line| serde_json::from_str::<UsageLine>(line).ok())
        .filter(|line| Source::of(line).is_some())
        .collect()
}

fn entries(lines: Vec<UsageLine>, windows: &[WorkerWindow]) -> Vec<Entry> {
    lines
        .into_iter()
        .map(|line| {
            let window = (line.kind == "worker")
                .then(|| {
                    let id = line.worker_id.as_deref()?;
                    window_position(windows, id, instant(&line.at)?)
                })
                .flatten();
            Entry { line, window }
        })
        .collect()
}

fn unmetered_cursors<'a>(windows: &'a [WorkerWindow], entries: &[Entry]) -> Vec<&'a WorkerWindow> {
    let joined: HashSet<usize> = entries.iter().filter_map(|entry| entry.window).collect();
    windows
        .iter()
        .enumerate()
        .filter(|(position, window)| {
            window.harness == HarnessId::Cursor.as_str() && !joined.contains(position)
        })
        .map(|(_, window)| window)
        .collect()
}

fn build<K: Ord>(
    entries: &[Entry],
    unmetered: Vec<K>,
    key: impl Fn(&Entry) -> Option<K>,
    label: impl Fn(&K) -> String,
    priced: &dyn Fn(&UsageLine) -> Usd,
) -> Vec<Row> {
    let mut groups: BTreeMap<K, (Vec<&UsageLine>, usize)> = BTreeMap::new();
    for entry in entries {
        if let Some(key) = key(entry) {
            groups.entry(key).or_default().0.push(&entry.line);
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
                .map(|line| Tokens::of(line))
                .fold(Tokens::default(), Tokens::plus),
            usd: lines
                .iter()
                .map(|line| priced(line))
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
    priced: &dyn Fn(&UsageLine) -> Usd,
) -> Vec<Row> {
    let cursors = unmetered_cursors(windows, entries);
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

pub fn costs_by_view(view: View, inputs: &Inputs, pricing: &Pricing) -> String {
    let grouping = match view {
        View::HarnessModel => return costs_view(inputs.usage, pricing),
        View::Harness => Grouping::Harness,
        View::Role => Grouping::Role,
        View::Ticket => Grouping::Ticket,
    };
    let windows = parse_windows(inputs.workers);
    let entries = entries(parse_lines(inputs.usage), &windows);
    let table = builtin_prices(pricing.provider);
    let priced = |line: &UsageLine| computed_usd(line, pricing, &table);
    let rows = rows(grouping, &entries, &windows, &priced);
    if rows.is_empty() {
        return NO_USAGE.to_string();
    }
    markdown(grouping, &rows)
}

#[cfg(test)]
mod tests;
