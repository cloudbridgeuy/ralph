mod anchors;
#[cfg(test)]
mod anchors_tests;
mod tokens;
#[cfg(test)]
mod tokens_tests;

use crate::chat_memory::{Entry, TagState};
use anchors::{pick_anchors, Anchor};
use tokens::document_frequency;

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
}

pub fn build_decay(entries: &[Entry], tags: &TagState, config: &DecayConfig) -> Decay {
    let frequency = document_frequency(entries);
    Decay {
        anchors: pick_anchors(entries, tags, &frequency, config.max_anchors),
    }
}
