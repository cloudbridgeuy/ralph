mod ancestors;
mod anchors;
#[cfg(test)]
mod anchors_tests;
mod survival;
#[cfg(test)]
mod survival_tests;
mod tokens;
#[cfg(test)]
mod tokens_tests;

use crate::chat_memory::{Entry, TagState, Tree};
use ancestors::ancestors;
use anchors::Group;
use anchors::{pick_anchors, Anchor};
use std::collections::BTreeMap;
use survival::{observe, pointer_cell, pointer_of, tally, Cell, Observation, Pointer, Table};
use tokens::document_frequency;

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
    pub observations: Vec<Observation>,
    pub pointers: Vec<Pointer>,
    pub survival: BTreeMap<Group, Table>,
    pub pointer_rates: BTreeMap<Group, Cell>,
}

pub fn build_decay(entries: &[Entry], tree: &Tree, tags: &TagState, config: &DecayConfig) -> Decay {
    let frequency = document_frequency(entries);
    let anchors = pick_anchors(entries, tags, &frequency, config.max_anchors);
    let mut observations = Vec::new();
    let mut pointers = Vec::new();
    for anchor in &anchors {
        let found = ancestors(anchor.id, entries.len(), tree);
        observations.extend(observe(anchor, &found));
        pointers.extend(pointer_of(anchor, &found));
    }
    let survival = GROUPS
        .iter()
        .map(|group| {
            (
                *group,
                tally(observations.iter().filter(|o| o.group == *group)),
            )
        })
        .collect();
    let pointer_rates = GROUPS
        .iter()
        .map(|group| {
            (
                *group,
                pointer_cell(pointers.iter().filter(|p| p.group == *group)),
            )
        })
        .collect();
    Decay {
        anchors,
        observations,
        pointers,
        survival,
        pointer_rates,
    }
}
