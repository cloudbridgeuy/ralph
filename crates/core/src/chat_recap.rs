use crate::chat_memory::{Entry, Role};
use crate::inbox::age;
use crate::workers_view::cut;
use chrono::{DateTime, Utc};

const MARKUP: &[char] = &['*', '_', '-', '>', '#', ' '];

pub struct RecapFacts<'a> {
    pub entries: &'a [Entry],
    pub now: DateTime<Utc>,
    pub workers: usize,
    pub unpushed: usize,
    pub width: usize,
}

pub fn recap(facts: &RecapFacts) -> Vec<String> {
    let readable = || {
        facts
            .entries
            .iter()
            .rev()
            .filter(|entry| !entry.is_corrupt())
    };
    let (Some(last), Some(reply)) = (
        readable().next(),
        readable().find(|entry| entry.role == Role::Assistant),
    ) else {
        return Vec::new();
    };
    let asked = readable()
        .find(|entry| entry.role == Role::User)
        .and_then(|entry| {
            entry
                .text
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
        });
    [
        Some(format!("last session · {} ago", age(&last.at, facts.now))),
        asked.map(|line| format!("  you: {line}")),
        next_step(&reply.text).map(|text| format!("  next: {text}")),
        live_line(facts.workers, facts.unpushed),
    ]
    .into_iter()
    .flatten()
    .map(|line| cut(&line, facts.width))
    .collect()
}

fn live_line(workers: usize, unpushed: usize) -> Option<String> {
    let noun = if workers == 1 { "worker" } else { "workers" };
    let parts: Vec<String> = [
        (workers > 0).then(|| format!("{workers} {noun}")),
        (unpushed > 0).then(|| format!("{unpushed} unpushed")),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| format!("  live: {}", parts.join(" · ")))
}

pub fn next_step(reply: &str) -> Option<&str> {
    reply.lines().rev().find_map(next_text)
}

fn next_text(line: &str) -> Option<&str> {
    let line = line.trim_start_matches(MARKUP);
    let rest = strip_word(line, "next")?;
    let rest = strip_word(rest, " step").unwrap_or(rest);
    let text = rest
        .trim_start_matches(MARKUP)
        .strip_prefix(':')?
        .trim_matches(MARKUP);
    (!text.is_empty()).then_some(text)
}

fn strip_word<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let head = text.get(..word.len())?;
    head.eq_ignore_ascii_case(word)
        .then(|| text.get(word.len()..))
        .flatten()
}

#[cfg(test)]
mod tests;
