use super::tree::{entry_line, node_line};
use super::{frozen_block, tail_lines, Entry, Frozen, Node, Tree};
use crate::chat::{resolve_frozen_window, VIEW_LINES};
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::str::FromStr;

/// Zero explicitly restores summary-only history; a verbatim budget is positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "usize", into = "usize")]
pub enum RecentHistory {
    Summarized,
    Verbatim(NonZeroUsize),
}

impl Default for RecentHistory {
    fn default() -> Self {
        Self::from(4096)
    }
}

impl From<usize> for RecentHistory {
    fn from(tokens: usize) -> Self {
        match NonZeroUsize::new(tokens) {
            Some(tokens) => Self::Verbatim(tokens),
            None => Self::Summarized,
        }
    }
}

impl From<RecentHistory> for usize {
    fn from(history: RecentHistory) -> Self {
        match history {
            RecentHistory::Summarized => 0,
            RecentHistory::Verbatim(tokens) => tokens.get(),
        }
    }
}

impl FromStr for RecentHistory {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value
            .parse::<usize>()
            .map(Self::from)
            .map_err(|_| "recent history tokens must be a non-negative whole number".to_string())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HistoryPolicy {
    window: usize,
    recent: RecentHistory,
}

impl HistoryPolicy {
    /// Parse configuration once before building views. Fields stay private so
    /// callers cannot construct a zero or out-of-range frozen window.
    pub fn parse(window: Option<usize>, recent: Option<RecentHistory>) -> Result<Self, String> {
        Ok(Self {
            window: resolve_frozen_window(None, window)?,
            recent: recent.unwrap_or_default(),
        })
    }
}

pub struct HistoryView {
    pub frozen: Frozen,
    pub tail: Vec<String>,
}

/// Charge one estimated token per UTF-8 byte of the rendered record, including
/// its ID, role, timestamp and trailing newline. This intentionally conservative
/// estimate avoids assuming English prose or a particular model's tokenizer.
fn record_cost(i: usize, entry: &Entry) -> usize {
    entry_line(i, entry, "")
        .len()
        .saturating_add(entry.text.len())
        .saturating_add(1)
}

fn verbatim_start(entries: &[Entry], recent: RecentHistory) -> usize {
    let RecentHistory::Verbatim(tokens) = recent else {
        return entries.len();
    };
    let mut remaining = tokens.get();
    let mut start = entries.len();
    for (i, entry) in entries.iter().enumerate().rev() {
        let Some(rest) = remaining.checked_sub(record_cost(i, entry)) else {
            break;
        };
        remaining = rest;
        start = i;
    }
    start
}

/// Keep a complete, contiguous suffix verbatim. Older messages retain their
/// summary representation. An oversized record ends the suffix: never truncate
/// it or skip it to include older raw messages instead.
pub fn history_view(
    entries: &[Entry],
    tree: &Tree,
    policy: HistoryPolicy,
    pinned: Option<Frozen>,
) -> HistoryView {
    let start = verbatim_start(entries, policy.recent);
    // Round the historical boundary down to a frozen window. As the suffix
    // slides, the prefix can remain byte-stable until this boundary rolls over.
    // If the newest record itself is oversized, leave it outside the frozen
    // prefix so its summary (or source pointer) remains visible even at rollover.
    let historical_end = match policy.recent {
        RecentHistory::Verbatim(_) if start == entries.len() => start.saturating_sub(1),
        _ => start,
    };
    let frozen = frozen_block(
        &entries[..historical_end],
        tree,
        VIEW_LINES,
        policy.window,
        pinned,
    );
    let mut tail = match policy.recent {
        RecentHistory::Summarized => tail_lines(entries, tree, frozen.end),
        RecentHistory::Verbatim(_) => (frozen.end..start)
            .map(|i| {
                node_line(entries, tree, Node::leaf(i)).unwrap_or_else(|| {
                    format!("#{i} not compressed yet (zoom {i} for original text)")
                })
            })
            .collect(),
    };
    tail.extend(
        entries[start..]
            .iter()
            .enumerate()
            .map(|(offset, entry)| entry_line(start + offset, entry, &entry.text)),
    );
    HistoryView { frozen, tail }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::fixtures::entry;
    use crate::chat_memory::pending;

    fn policy(window: usize, budget: usize) -> HistoryPolicy {
        HistoryPolicy::parse(Some(window), Some(budget.into())).unwrap()
    }

    fn summarized(entries: &[Entry]) -> Tree {
        pending(entries, &Tree::new())
            .into_iter()
            .map(|node| (node, format!("summary {}", node.id())))
            .collect()
    }

    #[test]
    fn settings_parse_into_known_modes_and_reject_invalid_windows() {
        assert_eq!("0".parse(), Ok(RecentHistory::Summarized));
        assert_eq!("4096".parse(), Ok(RecentHistory::default()));
        for invalid in ["-1", "1.5", "many", ""] {
            assert!(invalid.parse::<RecentHistory>().is_err());
        }
        assert!(HistoryPolicy::parse(Some(0), None).is_err());
        assert!(HistoryPolicy::parse(Some(VIEW_LINES), None).is_err());
        let default = HistoryPolicy::parse(None, None).unwrap();
        assert_eq!(default.window, 32);
        assert_eq!(default.recent, RecentHistory::default());
    }

    #[test]
    fn empty_history_has_no_context() {
        let view = history_view(&[], &Tree::new(), policy(2, 4096), None);
        assert!(view.frozen.lines.is_empty());
        assert!(view.tail.is_empty());
    }

    #[test]
    fn preserves_long_multiline_and_unicode_messages_without_a_summary() {
        let mut entries = vec![entry(0, 600), entry(1, 600)];
        entries[0].text = "ñandú\n  café\n".repeat(60);
        let budget = entries
            .iter()
            .enumerate()
            .map(|(i, e)| record_cost(i, e))
            .sum();
        let view = history_view(&entries, &Tree::new(), policy(2, budget), None);
        assert_eq!(view.frozen.end, 0);
        assert_eq!(view.tail.len(), 2);
        assert_eq!(view.tail[0], entry_line(0, &entries[0], &entries[0].text));
        assert_eq!(view.tail[1], entry_line(1, &entries[1], &entries[1].text));
        assert_eq!(verbatim_start(&entries, budget.into()), 0);
        assert_eq!(verbatim_start(&entries, (budget - 1).into()), 1);
    }

    #[test]
    fn oversized_newest_message_uses_summary_or_source_pointer() {
        let entries = vec![entry(0, 5), entry(1, 600)];
        let tree = summarized(&entries);
        let view = history_view(&entries, &tree, policy(3, 100), None);
        assert_eq!(verbatim_start(&entries, 100.into()), 2);
        assert_eq!(view.tail[1], entry_line(1, &entries[1], "summary 1"));
        let pending = history_view(&entries, &Tree::new(), policy(3, 100), None);
        assert_eq!(
            pending.tail[1],
            "#1 not compressed yet (zoom 1 for original text)"
        );
    }

    #[test]
    fn oversized_message_stops_suffix_without_skipping_backwards() {
        let entries = vec![entry(0, 5), entry(1, 600), entry(2, 5)];
        let budget = record_cost(0, &entries[0]) + record_cost(2, &entries[2]);
        let view = history_view(&entries, &summarized(&entries), policy(4, budget), None);
        assert_eq!(verbatim_start(&entries, budget.into()), 2);
        assert_eq!(view.tail.len(), 3);
        assert!(view.tail[1].ends_with("summary 1"));
        assert_eq!(view.tail[2], entry_line(2, &entries[2], &entries[2].text));
    }

    #[test]
    fn oversized_newest_record_keeps_its_source_pointer_at_window_rollover() {
        let entries: Vec<_> = (0..32).map(|i| entry(i, 600)).collect();
        let view = history_view(&entries, &Tree::new(), policy(32, 1), None);
        assert_eq!(view.frozen.end, 0);
        assert_eq!(view.tail.len(), 32);
        assert_eq!(
            view.tail[31],
            "#31 not compressed yet (zoom 31 for original text)"
        );
    }

    #[test]
    fn disabling_verbatim_history_keeps_the_existing_split() {
        let entries: Vec<_> = (0..10).map(|i| entry(i, 600)).collect();
        let tree = summarized(&entries);
        let old = frozen_block(&entries, &tree, VIEW_LINES, 3, None);
        let view = history_view(&entries, &tree, policy(3, 0), None);
        assert_eq!(view.frozen, old);
        assert_eq!(view.tail, tail_lines(&entries, &tree, old.end));
    }

    #[test]
    fn pinned_prefix_stays_stable_then_rolls_over_without_overlap() {
        let entries: Vec<_> = (0..50).map(|i| entry(i, 600)).collect();
        let tree = summarized(&entries);
        let options = policy(4, record_cost(49, &entries[49]) * 2);
        let mut pinned = None;
        let mut previous: Option<Frozen> = None;
        for end in 12..=50 {
            let view = history_view(&entries[..end], &tree, options, pinned.take());
            let raw_start = verbatim_start(&entries[..end], options.recent);
            assert_eq!(view.frozen.end, raw_start / 4 * 4);
            if let Some(previous) = &previous {
                if previous.end == view.frozen.end {
                    assert_eq!(previous.lines, view.frozen.lines);
                }
            }
            let ids: Vec<_> = view
                .tail
                .iter()
                .map(|line| line.split(' ').next().unwrap())
                .collect();
            let expected: Vec<_> = (view.frozen.end..end).map(|i| format!("#{i}")).collect();
            assert_eq!(ids, expected);
            assert_eq!(
                view.tail.last().unwrap(),
                &entry_line(end - 1, &entries[end - 1], &entries[end - 1].text)
            );
            previous = Some(view.frozen.clone());
            pinned = Some(view.frozen);
        }
    }

    #[test]
    fn long_history_covers_every_message_once_and_bounds_verbatim_bytes() {
        let entries: Vec<_> = (0..1000).map(|i| entry(i, 600)).collect();
        let budget = 4096;
        let view = history_view(&entries, &summarized(&entries), policy(32, budget), None);
        assert!(view.frozen.lines.iter().any(|line| line.starts_with("#0-")));
        let mut next = 0;
        for line in view.frozen.lines.iter().chain(&view.tail) {
            let id = line.split(' ').next().unwrap().strip_prefix('#').unwrap();
            let node = Node::parse(id).unwrap();
            assert_eq!(node.lo, next, "gap or overlap at {id}");
            next = node.hi;
        }
        assert_eq!(next, entries.len());
        let start = verbatim_start(&entries, budget.into());
        let raw = &view.tail[view.tail.len() - (entries.len() - start)..];
        assert!(raw.iter().map(|line| line.len() + 1).sum::<usize>() <= budget);
        assert_eq!(
            raw[0],
            entry_line(start, &entries[start], &entries[start].text)
        );
        assert!(raw.len() > 1);
    }
}
