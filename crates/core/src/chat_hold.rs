use crate::chat_status::Status;
use crate::inbox::{Entry, Position};
use crate::schedule::watch_id;
use crate::schedule_worker::source_matches_worker;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ops::Range;

const HOLD: &str = "/hold";
const CTRL_Q: u8 = 0x11;
const CTRL_D: u8 = 0x04;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldFilter {
    Any,
    Source(String),
    Set(Vec<String>),
}

impl HoldFilter {
    pub fn parse(args: &str) -> Option<Self> {
        let mut sources: Vec<String> = Vec::new();
        for token in args.split_whitespace() {
            if !sources.iter().any(|source| source == token) {
                sources.push(token.to_string());
            }
        }
        match sources.len() {
            0 => Some(HoldFilter::Any),
            1 => sources.pop().map(HoldFilter::Source),
            _ => Some(HoldFilter::Set(sources)),
        }
    }

    pub fn label(&self) -> Cow<'_, str> {
        match self {
            HoldFilter::Any => Cow::Borrowed("anyone"),
            HoldFilter::Source(source) => Cow::Borrowed(source),
            HoldFilter::Set(sources) => Cow::Owned(sources.join(", ")),
        }
    }

    pub fn render(&self) -> String {
        match self {
            HoldFilter::Any => HOLD.to_string(),
            HoldFilter::Source(source) => format!("{HOLD} {source}"),
            HoldFilter::Set(sources) => format!("{HOLD} {}", sources.join(" ")),
        }
    }

    pub fn matches(&self, entry: &Entry) -> bool {
        match self {
            HoldFilter::Any => true,
            HoldFilter::Source(source) => source_matches_worker(source, &entry.report.worker),
            HoldFilter::Set(sources) => sources
                .iter()
                .any(|source| source_matches_worker(source, &entry.report.worker)),
        }
    }
}

pub fn first_match(entries: &[Entry], filter: &HoldFilter) -> Option<Position> {
    entries
        .iter()
        .position(|entry| filter.matches(entry))
        .and_then(Position::at_index)
}

pub fn redraw(bar: &str, painted: usize, columns: usize) -> String {
    let up = match painted.div_ceil(columns.max(1)).saturating_sub(1) {
        0 => String::new(),
        rows => format!("\x1b[{rows}A"),
    };
    format!("\r{up}\x1b[J{}", bar.trim_end_matches('\n'))
}

pub fn hold_targets(status: &Status) -> BTreeSet<String> {
    status
        .workers
        .iter()
        .map(|worker| worker.id.clone())
        .chain(status.watches.iter().map(|watch| watch.id.clone()))
        .chain(status.schedule.iter().map(|task| watch_id(&task.id)))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldCompletion {
    pub span: Range<usize>,
    pub values: Vec<String>,
}

pub fn hold_completions(line: &str, cursor: usize, status: &Status) -> Option<HoldCompletion> {
    let args = line.get(..cursor)?.trim_start().strip_prefix("/hold")?;
    let partial = args.trim_start();
    if partial.len() == args.len() || partial.contains(char::is_whitespace) {
        return None;
    }
    let candidates: BTreeSet<String> = hold_targets(status)
        .into_iter()
        .filter(|candidate| candidate.starts_with(partial))
        .collect();
    Some(HoldCompletion {
        span: cursor - partial.len()..cursor,
        values: candidates.into_iter().collect(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leave;

pub fn hold_key(bytes: &[u8]) -> Option<Leave> {
    bytes
        .iter()
        .any(|byte| matches!(*byte, CTRL_Q | CTRL_D))
        .then_some(Leave)
}

#[cfg(test)]
mod tests;
