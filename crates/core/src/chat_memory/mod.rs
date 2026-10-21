mod archive;
#[cfg(test)]
mod corrupt_tests;
mod index;
mod log;
mod lookup;
mod nap;
mod notice;
mod range;
mod search;
mod tools;
mod tree;
mod usage;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use archive::{refresh, write_plan, Archive, ByteSource, IndexWrite};
pub use index::{
    index_temp_prefix, is_index_temp, Index, LogIndex, NodeSlot, Slot, TreeIndex, INDEX_TEMP_RAND,
    INDEX_TEMP_SUFFIX, LOG_INDEX_FILE, TREE_INDEX_FILE,
};
pub use log::{
    encode_entry, encode_record, entry_or_corrupt, log_tail, parse_log, parse_tree, LogTail,
    CORRUPT_TEXT,
};
pub use lookup::{recall, zoom};
pub use nap::{
    check_summary, commit_decision, fallback_summary, nap_source, nap_step, nap_summary,
    retry_prompt, Commit, NapStep, FALLBACK_MARK, MAX_NAP_ATTEMPTS,
};
pub use range::{read_range, Mode, RangeRequest, Read};
pub use search::{parse_search_args, search, SearchRequest};
pub use tools::{finish_call, handle_request, parse_tool_call, tool_definitions, Outcome, Tool};
pub use tree::{
    cover, estimate_tokens, pending, split_view, tail_lines, verbatim_start, ViewSplit,
};
pub use usage::{
    encode_usage, reply_entries, trigger_entries, usage_record, view_stats, CallKind, Trigger,
    UsageRecord,
};

pub const SUMMARY_BYTES: usize = 512;
pub const UNAVAILABLE_MARK: &str = "[not summarized] ";
pub const SERVER_NAME: &str = "ralph-chat";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Worker,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Worker => "worker",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NewEntry {
    pub role: Role,
    pub at: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Entry {
    pub role: Role,
    pub at: String,
    pub text: String,
    #[serde(skip)]
    pub corrupt: bool,
}

impl Entry {
    pub fn corrupt() -> Entry {
        Entry {
            role: Role::Worker,
            at: String::new(),
            text: CORRUPT_TEXT.to_string(),
            corrupt: true,
        }
    }

    pub fn is_corrupt(&self) -> bool {
        self.corrupt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Node {
    pub lo: usize,
    pub hi: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    lo: usize,
    hi: usize,
    text: String,
}

pub type Tree = HashMap<Node, String>;

/// Entries `[start, start + entries.len())` of a log that holds `total`.
#[derive(Debug, Clone, Copy)]
pub struct Window<'a> {
    pub start: usize,
    pub total: usize,
    pub entries: &'a [Entry],
}

impl<'a> Window<'a> {
    pub fn full(entries: &'a [Entry]) -> Self {
        Window {
            start: 0,
            total: entries.len(),
            entries,
        }
    }

    pub fn get(&self, i: usize) -> Option<&'a Entry> {
        i.checked_sub(self.start).and_then(|k| self.entries.get(k))
    }
}

impl Node {
    pub fn new(lo: usize, hi: usize) -> Result<Node, String> {
        let size = hi.saturating_sub(lo);
        if size == 0 || !size.is_power_of_two() || !lo.is_multiple_of(size) {
            return Err(format!(
                "[{lo},{hi}) is not a block: expected an aligned power-of-two range."
            ));
        }
        Ok(Node { lo, hi })
    }

    pub fn parse(s: &str) -> Result<Node, String> {
        let bad = || format!("'{s}' is not a node id. Use an index like 5 or a range like 4-7.");
        let number = |p: &str| {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            p.parse::<usize>().map_err(|_| bad())
        };
        let node = match s.split_once('-') {
            None => {
                let lo = number(s)?;
                Node::new(lo, lo.checked_add(1).ok_or_else(bad)?)
            }
            Some((a, b)) => {
                let (lo, last) = (number(a)?, number(b)?);
                Node::new(lo, last.checked_add(1).ok_or_else(bad)?)
            }
        };
        node.map_err(|_| format!("{s} is not a node. Copy an id printed by the view, like 16-31."))
    }

    pub fn id(&self) -> String {
        if self.size() == 1 {
            self.lo.to_string()
        } else {
            format!("{}-{}", self.lo, self.hi - 1)
        }
    }

    pub fn size(&self) -> usize {
        self.hi - self.lo
    }

    fn halves(&self) -> (Node, Node) {
        let mid = (self.lo + self.hi) / 2;
        (
            Node {
                lo: self.lo,
                hi: mid,
            },
            Node {
                lo: mid,
                hi: self.hi,
            },
        )
    }

    fn leaf(i: usize) -> Node {
        Node { lo: i, hi: i + 1 }
    }
}

#[cfg(test)]
mod fixtures {
    use super::{encode_entry, Entry, NewEntry, Node, Role};

    pub fn n(lo: usize, hi: usize) -> Node {
        Node { lo, hi }
    }

    pub fn entry(i: usize, len: usize) -> Entry {
        Entry {
            role: if i.is_multiple_of(2) {
                Role::User
            } else {
                Role::Assistant
            },
            at: format!("2026-01-01T00:00:{:02}Z", i % 60),
            text: format!("m{i} {}", "x".repeat(len)),
            corrupt: false,
        }
    }

    pub fn written(entry: &Entry) -> String {
        encode_entry(&NewEntry {
            role: entry.role,
            at: entry.at.clone(),
            text: entry.text.clone(),
        })
        .unwrap()
    }

    pub fn texted(count: usize, text: impl Fn(usize) -> String) -> Vec<Entry> {
        (0..count)
            .map(|i| {
                let mut e = entry(i, 5);
                e.text = text(i);
                e
            })
            .collect()
    }
}
