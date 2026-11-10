pub mod chart;
pub mod model_view;
pub mod text;

use super::tickets::{window_position, WorkerWindow};
use super::window::instant;
use super::{builtin_prices, computed_usd, Source, Tokens, Usd, UNKNOWN};
use crate::chat_process::UsageLine;
use crate::wire::price::PriceTable;
use crate::wire::view::Pricing;
use crate::worker::HarnessId;
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};

pub const MODEL_MISSING: &str = "(model missing)";
pub const EST: &str = "est.";
pub const EXCLUDED: &str = "excluded";
pub const NO_DATA: &str = "no data";
pub const NO_PRICE: &str = "no price";
const HARNESS_COST_SOURCE: &str = "harness";
const TOTAL: &str = "total";

type SortKey = (u8, String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Main,
    Nap,
    Merge,
    Ask,
    Subagent,
    Worker,
}

impl Role {
    pub fn of(kind: &str) -> Option<Role> {
        match kind {
            "turn" | "ping" => Some(Role::Main),
            "nap" | "tag" => Some(Role::Nap),
            "merge" => Some(Role::Merge),
            "ask" => Some(Role::Ask),
            "subagent" => Some(Role::Subagent),
            "worker" => Some(Role::Worker),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
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

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub store: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub ticket: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Kind,
    Store,
    Harness,
    Model,
    Ticket,
    Run,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub line: UsageLine,
    pub source: Source,
    pub role: Role,
    pub at: DateTime<Utc>,
    pub reported: bool,
    pub window: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct Ledger {
    pub entries: Vec<Entry>,
    pub malformed: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Group {
    pub key: String,
    pub metered: usize,
    pub unmetered: usize,
    pub tokens: Tokens,
    pub reported: Usd,
    pub est: Usd,
    pub excluded: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub groups: Vec<Group>,
    pub total: Group,
    pub malformed: usize,
}

#[derive(Clone, Copy)]
enum Series {
    Reported,
    Est,
    Excluded,
}

#[derive(Deserialize)]
struct CostSourceTag {
    cost_source: Option<String>,
}

pub fn parse_bound<Tz: TimeZone>(text: &str, zone: &Tz) -> Option<DateTime<Utc>> {
    if let Some(at) = instant(text) {
        return Some(at);
    }
    let midnight = NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()?
        .and_time(NaiveTime::MIN);
    zone.from_local_datetime(&midnight)
        .earliest()
        .map(|at| at.with_timezone(&Utc))
}

pub fn is_excluded(line: &UsageLine) -> bool {
    line.harness.as_deref() == Some(HarnessId::Codex.as_str())
}

pub fn label_excluded(cost: String, excluded: usize, metered: usize) -> String {
    if excluded == 0 {
        cost
    } else if excluded == metered {
        EXCLUDED.to_string()
    } else {
        format!("{cost} + {EXCLUDED}")
    }
}

pub(crate) fn partition<'a, K: Ord>(
    entries: impl Iterator<Item = &'a Entry>,
    key: impl Fn(&Entry) -> K,
) -> BTreeMap<K, Vec<&'a Entry>> {
    let mut groups: BTreeMap<K, Vec<&Entry>> = BTreeMap::new();
    for entry in entries {
        groups.entry(key(entry)).or_default().push(entry);
    }
    groups
}

impl Group {
    pub fn usd(&self) -> Usd {
        self.reported.plus(self.est)
    }

    pub fn cache_hit_rate(&self) -> Option<f64> {
        let tokens = self.tokens;
        let prompt = tokens.cache_read as f64 + tokens.cache_write as f64 + tokens.input as f64;
        (prompt > 0.0).then(|| tokens.cache_read as f64 / prompt)
    }

    pub fn state(&self) -> Option<&'static str> {
        if self.metered == 0 {
            Some(NO_DATA)
        } else if self.all_excluded() {
            Some(EXCLUDED)
        } else if self.usd().known.is_none() {
            Some(NO_PRICE)
        } else {
            None
        }
    }

    pub fn line(&self) -> String {
        match self.state() {
            Some(state) => format!("{}: {state}", self.key),
            None => format!("{} {}", self.key, self.cost_label()),
        }
    }

    pub fn cost_label(&self) -> String {
        self.with_excluded(self.cost_text())
    }

    pub fn all_excluded(&self) -> bool {
        self.excluded > 0 && self.excluded == self.metered
    }

    pub fn with_excluded(&self, cost: String) -> String {
        label_excluded(cost, self.excluded, self.metered)
    }

    fn cost_text(&self) -> String {
        let amount = |usd: Usd| {
            usd.known
                .map(|known| format!("${known:.2}{}", if usd.missing { "+" } else { "" }))
        };
        match (amount(self.reported), amount(self.est)) {
            (Some(reported), Some(est)) => format!("{reported} + {est} {EST}"),
            (Some(reported), None) => reported,
            (None, Some(est)) => format!("{est} {EST}"),
            (None, None) => String::new(),
        }
    }

    fn add(&mut self, entry: &Entry, series: Series, usd: Usd) {
        self.metered += 1;
        self.tokens = self.tokens.plus(Tokens::of(&entry.line));
        match series {
            Series::Reported => self.reported = self.reported.plus(usd),
            Series::Est => self.est = self.est.plus(usd),
            Series::Excluded => self.excluded += 1,
        }
    }

    fn merged(self, other: &Group) -> Group {
        Group {
            key: self.key,
            metered: self.metered + other.metered,
            unmetered: self.unmetered + other.unmetered,
            tokens: self.tokens.plus(other.tokens),
            reported: self.reported.plus(other.reported),
            est: self.est.plus(other.est),
            excluded: self.excluded + other.excluded,
        }
    }
}

enum Parsed {
    Entry(Box<Entry>),
    Skipped,
    Malformed,
}

fn parsed(text: &str, windows: &[WorkerWindow]) -> Parsed {
    let Ok(line) = serde_json::from_str::<UsageLine>(text) else {
        return Parsed::Malformed;
    };
    let Some(at) = instant(&line.at) else {
        return Parsed::Malformed;
    };
    let (Some(source), Some(role)) = (Source::of(&line), Role::of(&line.kind)) else {
        return Parsed::Skipped;
    };
    let from_harness = serde_json::from_str::<CostSourceTag>(text)
        .is_ok_and(|tag| tag.cost_source.as_deref() == Some(HARNESS_COST_SOURCE));
    let reported = from_harness && line.cost_usd.is_some() && role != Role::Main;
    let window = (role == Role::Worker)
        .then(|| window_position(windows, line.worker_id.as_deref()?, at))
        .flatten();
    Parsed::Entry(Box::new(Entry {
        line,
        source,
        role,
        at,
        reported,
        window,
    }))
}

pub fn parse(usage: &str, windows: &[WorkerWindow]) -> Ledger {
    let mut ledger = Ledger::default();
    for text in usage.lines().filter(|text| !text.trim().is_empty()) {
        match parsed(text, windows) {
            Parsed::Entry(entry) => ledger.entries.push(*entry),
            Parsed::Skipped => {}
            Parsed::Malformed => ledger.malformed += 1,
        }
    }
    ledger
}

fn ticket_of<'a>(entry: &Entry, windows: &'a [WorkerWindow]) -> Option<&'a str> {
    entry
        .window
        .and_then(|position| windows.get(position))
        .and_then(|window| window.ticket.as_deref())
}

fn wants(wanted: Option<&str>, actual: Option<&str>) -> bool {
    wanted.is_none_or(|wanted| actual == Some(wanted))
}

fn admits(entry: &Entry, filter: &Filter, windows: &[WorkerWindow]) -> bool {
    filter.since.is_none_or(|since| entry.at >= since)
        && filter.until.is_none_or(|until| entry.at < until)
        && wants(filter.store.as_deref(), entry.line.store.as_deref())
        && wants(filter.harness.as_deref(), Some(entry.source.label()))
        && wants(filter.model.as_deref(), entry.line.model.as_deref())
        && wants(filter.kind.as_deref(), Some(entry.role.label()))
        && wants(
            filter.ticket.as_deref(),
            Some(ticket_of(entry, windows).unwrap_or(UNKNOWN)),
        )
}

fn named(value: Option<&str>) -> SortKey {
    value.map_or_else(|| (1, UNKNOWN.to_string()), |value| (0, value.to_string()))
}

fn source_key(source: &Source) -> SortKey {
    let rank = match source {
        Source::Main => 0,
        Source::Nap => 1,
        Source::Ask => 2,
        Source::Subagent => 3,
        Source::Worker(_) => 4,
    };
    (rank, source.label().to_string())
}

fn role_key(role: Role) -> SortKey {
    (role as u8, role.label().to_string())
}

fn key_of(dim: Dim, entry: &Entry, windows: &[WorkerWindow]) -> SortKey {
    match dim {
        Dim::Kind => role_key(entry.role),
        Dim::Harness => source_key(&entry.source),
        Dim::Store => named(entry.line.store.as_deref()),
        Dim::Model => (
            0,
            entry
                .line
                .model
                .clone()
                .unwrap_or_else(|| MODEL_MISSING.to_string()),
        ),
        Dim::Ticket => named(ticket_of(entry, windows)),
        Dim::Run => named(entry.line.run.as_deref()),
    }
}

fn admits_window(window: &WorkerWindow, filter: &Filter) -> bool {
    filter.since.is_none_or(|since| window.start >= since)
        && filter.until.is_none_or(|until| window.start < until)
        && filter.store.is_none()
        && filter.model.is_none()
        && wants(filter.harness.as_deref(), Some(HarnessId::Cursor.as_str()))
        && wants(filter.kind.as_deref(), Some(Role::Worker.label()))
        && wants(
            filter.ticket.as_deref(),
            Some(window.ticket.as_deref().unwrap_or(UNKNOWN)),
        )
}

fn unmetered_keys(
    dim: Dim,
    ledger: &Ledger,
    windows: &[WorkerWindow],
    filter: &Filter,
) -> Vec<SortKey> {
    let joined: HashSet<usize> = ledger
        .entries
        .iter()
        .filter_map(|entry| entry.window)
        .collect();
    windows
        .iter()
        .enumerate()
        .filter(|(position, window)| {
            window.harness == HarnessId::Cursor.as_str()
                && !joined.contains(position)
                && admits_window(window, filter)
        })
        .filter_map(|(_, window)| match dim {
            Dim::Harness => Some(source_key(&Source::Worker(window.harness.clone()))),
            Dim::Kind => Some(role_key(Role::Worker)),
            Dim::Ticket => Some(named(window.ticket.as_deref())),
            Dim::Store | Dim::Model | Dim::Run => None,
        })
        .collect()
}

fn priced(entry: &Entry, pricing: &Pricing, table: &PriceTable) -> (Series, Usd) {
    if is_excluded(&entry.line) {
        return (Series::Excluded, Usd::default());
    }
    match (entry.reported, entry.line.cost_usd) {
        (true, Some(cost)) => (Series::Reported, Usd::reported(cost)),
        _ => (Series::Est, computed_usd(&entry.line, pricing, table)),
    }
}

fn slot(groups: &mut BTreeMap<SortKey, Group>, key: SortKey) -> &mut Group {
    let label = key.1.clone();
    groups.entry(key).or_insert_with(|| Group {
        key: label,
        ..Group::default()
    })
}

fn report(
    ledger: &Ledger,
    pricing: &Pricing,
    unmetered: Vec<SortKey>,
    admit: impl Fn(&Entry) -> bool,
    key: impl Fn(&Entry) -> SortKey,
) -> Report {
    let table = builtin_prices(pricing.provider);
    let mut groups: BTreeMap<SortKey, Group> = BTreeMap::new();
    for key in unmetered {
        slot(&mut groups, key).unmetered += 1;
    }
    let admitted = ledger.entries.iter().filter(|entry| admit(entry));
    for (key, entries) in partition(admitted, key) {
        let group = slot(&mut groups, key);
        for entry in entries {
            let (series, usd) = priced(entry, pricing, &table);
            group.add(entry, series, usd);
        }
    }
    let groups: Vec<Group> = groups.into_values().collect();
    let total = groups.iter().fold(
        Group {
            key: TOTAL.to_string(),
            ..Group::default()
        },
        Group::merged,
    );
    Report {
        groups,
        total,
        malformed: ledger.malformed,
    }
}

pub fn group_by(
    dim: Dim,
    ledger: &Ledger,
    windows: &[WorkerWindow],
    filter: &Filter,
    pricing: &Pricing,
) -> Report {
    report(
        ledger,
        pricing,
        unmetered_keys(dim, ledger, windows, filter),
        |entry| admits(entry, filter, windows),
        |entry| key_of(dim, entry, windows),
    )
}

pub fn daily<Tz: TimeZone>(
    ledger: &Ledger,
    windows: &[WorkerWindow],
    filter: &Filter,
    zone: &Tz,
    pricing: &Pricing,
) -> Report {
    report(
        ledger,
        pricing,
        Vec::new(),
        |entry| admits(entry, filter, windows),
        |entry| (0, entry.at.with_timezone(zone).date_naive().to_string()),
    )
}

#[cfg(test)]
mod tests;
