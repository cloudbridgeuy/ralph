use super::anchors::{Anchor, Group};
use super::survival::Cell;
use super::tokens::{text_words, Key};
use crate::chat_memory::search::rank_ids;
use crate::chat_memory::{parse_search_args, recall_found, Entry, Node, Tree};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bucket {
    One,
    TwoToFive,
    SixToTwenty,
    Over,
    Miss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Uncompressed,
    Unreachable,
    Reached { search: Bucket, recall: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub group: Group,
    pub last: usize,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReachTally {
    pub unreachable: usize,
    pub search: BTreeMap<Bucket, usize>,
    pub recall: Cell,
}

pub fn bucket(rank: Option<usize>) -> Bucket {
    match rank {
        Some(1) => Bucket::One,
        Some(2..=5) => Bucket::TwoToFive,
        Some(6..=20) => Bucket::SixToTwenty,
        Some(_) => Bucket::Over,
        None => Bucket::Miss,
    }
}

pub fn cover_ancestor(id: usize, cover: &[Node]) -> Option<Node> {
    cover
        .iter()
        .find(|node| node.lo <= id && id < node.hi)
        .copied()
}

fn surviving<'a>(keys: &'a [Key], summary: &str) -> Vec<&'a str> {
    let words = text_words(summary);
    keys.iter()
        .filter(|key| words.contains(&key.word))
        .map(|key| key.word.as_str())
        .collect()
}

pub fn search_rank(entries: &[Entry], id: usize, words: &[&str]) -> Option<usize> {
    let request = parse_search_args(&json!({ "query": words.join(" ") })).ok()?;
    rank_ids(entries, &request)
        .iter()
        .position(|hit| *hit == id)
        .map(|at| at + 1)
}

pub fn recall_position(entries: &[Entry], id: usize, words: &[&str]) -> Option<usize> {
    let pattern = words
        .iter()
        .map(|word| regex::escape(word))
        .collect::<Vec<_>>()
        .join("|");
    let found = recall_found(entries, &pattern).ok()?;
    found
        .shown
        .iter()
        .position(|hit| *hit == id)
        .map(|at| at + 1)
}

pub fn probe(anchor: &Anchor, entries: &[Entry], tree: &Tree, cover: &[Node]) -> Option<Probe> {
    if anchor.keys.is_empty() {
        return None;
    }
    let node = cover_ancestor(anchor.id, cover)?;
    let outcome = if node.size() == 1 {
        Outcome::Uncompressed
    } else {
        let words = tree
            .get(&node)
            .map(|summary| surviving(&anchor.keys, summary))
            .unwrap_or_default();
        if words.is_empty() {
            Outcome::Unreachable
        } else {
            Outcome::Reached {
                search: bucket(search_rank(entries, anchor.id, &words)),
                recall: recall_position(entries, anchor.id, &words).is_some(),
            }
        }
    };
    Some(Probe {
        group: anchor.group,
        last: node.hi - 1,
        outcome,
    })
}

pub fn reach_tally<'a>(probes: impl IntoIterator<Item = &'a Probe>) -> ReachTally {
    let mut tally = ReachTally::default();
    for probe in probes {
        match probe.outcome {
            Outcome::Uncompressed => {}
            Outcome::Unreachable => tally.unreachable += 1,
            Outcome::Reached { search, recall } => {
                *tally.search.entry(search).or_insert(0) += 1;
                tally.recall.add(recall);
            }
        }
    }
    tally
}
