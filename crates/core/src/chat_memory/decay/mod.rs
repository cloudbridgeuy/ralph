mod ancestors;
mod anchors;
#[cfg(test)]
mod anchors_tests;
mod era;
#[cfg(test)]
mod era_tests;
mod reach;
#[cfg(test)]
mod reach_tests;
mod render;
#[cfg(test)]
mod render_tests;
mod survival;
#[cfg(test)]
mod survival_tests;
mod tokens;
#[cfg(test)]
mod tokens_tests;

use crate::chat::{RECENT_HISTORY_TOKENS, VIEW_LINES};
use crate::chat_memory::tree::older_nodes;
use crate::chat_memory::{Entry, TagState, Tree};
use ancestors::ancestors;
use anchors::{pick_anchors, Anchor, Group};
use era::{section, Collected, Section};
use reach::{probe, Outcome};
use std::collections::BTreeMap;
use survival::{observe, pointer_of};
use tokens::document_frequency;

pub use render::render_decay;

const GROUPS: [Group; 2] = [Group::Tagged, Group::TopUp];
const DEFAULT_MAX_ANCHORS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecayConfig {
    pub max_anchors: usize,
}

impl Default for DecayConfig {
    fn default() -> DecayConfig {
        DecayConfig {
            max_anchors: DEFAULT_MAX_ANCHORS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decay {
    pub anchors: Vec<Anchor>,
    pub uncompressed: usize,
    pub keyless: BTreeMap<Group, usize>,
    pub sections: BTreeMap<Group, Section>,
}

pub fn build_decay(entries: &[Entry], tree: &Tree, tags: &TagState, config: &DecayConfig) -> Decay {
    let frequency = document_frequency(entries);
    let anchors = pick_anchors(entries, tags, &frequency, config.max_anchors);
    let mut observations = Vec::new();
    let mut pointers = Vec::new();
    let mut probes = Vec::new();
    let view_cover = older_nodes(entries, tree, VIEW_LINES, RECENT_HISTORY_TOKENS);
    for anchor in &anchors {
        let found = ancestors(anchor.id, entries.len(), tree);
        observations.extend(observe(anchor, &found));
        pointers.extend(pointer_of(anchor, &found));
        probes.extend(probe(anchor, entries, tree, &view_cover));
    }
    let uncompressed = probes
        .iter()
        .filter(|p| p.outcome == Outcome::Uncompressed)
        .count();
    let keyless = GROUPS
        .iter()
        .map(|group| {
            let count = anchors
                .iter()
                .filter(|a| a.group == *group && a.keys.is_empty())
                .count();
            (*group, count)
        })
        .collect();
    let data = Collected {
        observations: &observations,
        pointers: &pointers,
        probes: &probes,
    };
    let sections = GROUPS
        .iter()
        .map(|group| (*group, section(*group, &data, entries)))
        .collect();
    Decay {
        anchors,
        uncompressed,
        keyless,
        sections,
    }
}
