use crate::inbox::{Entry, Position};

const CTRL_Q: u8 = 0x11;
const CTRL_D: u8 = 0x04;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldFilter {
    Any,
    Source(String),
}

impl HoldFilter {
    pub fn parse(args: &str) -> Option<Self> {
        let mut tokens = args.split_whitespace();
        match (tokens.next(), tokens.next()) {
            (None, _) => Some(HoldFilter::Any),
            (Some(source), None) => Some(HoldFilter::Source(source.to_string())),
            _ => None,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            HoldFilter::Any => "any",
            HoldFilter::Source(source) => source,
        }
    }

    pub fn matches(&self, entry: &Entry) -> bool {
        match self {
            HoldFilter::Any => true,
            HoldFilter::Source(source) => entry.source == *source,
        }
    }
}

pub fn first_match(entries: &[Entry], filter: &HoldFilter) -> Option<Position> {
    entries
        .iter()
        .position(|entry| filter.matches(entry))
        .and_then(Position::at_index)
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
