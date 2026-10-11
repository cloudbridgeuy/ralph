mod log;
mod nap;
mod tools;
mod tree;
mod usage;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use log::{encode_entry, encode_record, parse_log, parse_tree};
pub use nap::{check_summary, nap_prompt, nap_summary, retry_prompt, MAX_NAP_ATTEMPTS};
pub use tools::{finish_call, handle_request, parse_tool_call, tool_definitions, Outcome, Tool};
pub use tree::{cover, pending, recall, view, zoom};
pub use usage::{encode_usage, usage_record, view_stats, CallKind, Trigger, UsageRecord};

pub const SUMMARY_BYTES: usize = 512;
pub const SERVER_NAME: &str = "ralph-chat";
pub const RECALL_CAP_BYTES: usize = 8192;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub role: Role,
    pub at: String,
    pub text: String,
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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct View {
    pub lines: Vec<String>,
    pub missing: Vec<Node>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Recall {
    pub lines: Vec<String>,
    pub hits: usize,
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
    use super::{Entry, Node, Role};

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
        }
    }
}
