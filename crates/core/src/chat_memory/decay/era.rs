use super::anchors::Group;
use super::reach::{reach_tally, Probe, ReachTally};
use super::survival::{pointer_cell, tally, Cell, Observation, Pointer, Table};
use crate::chat_memory::Entry;
use std::collections::BTreeMap;

const MID_ERA_FROM: &str = "2026-11-12";
const LATE_ERA_FROM: &str = "2026-11-13";
const DAY_CHARS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Era {
    Early,
    Mid,
    Late,
}

pub const ERAS: [Era; 3] = [Era::Early, Era::Mid, Era::Late];

impl Era {
    pub fn label(self) -> &'static str {
        match self {
            Era::Early => "before 2026-11-12",
            Era::Mid => "2026-11-12",
            Era::Late => "from 2026-11-13",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Block {
    pub survival: Table,
    pub pointer: Cell,
    pub reach: ReachTally,
}

impl Block {
    pub fn is_empty(&self) -> bool {
        self.survival.is_empty()
            && self.pointer.total == 0
            && self.reach.unreachable == 0
            && self.reach.search.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    pub overall: Block,
    pub eras: BTreeMap<Era, Block>,
}

#[derive(Debug, Clone, Copy)]
pub struct Collected<'a> {
    pub observations: &'a [Observation],
    pub pointers: &'a [Pointer],
    pub probes: &'a [Probe],
}

pub fn era_of(at: &str) -> Era {
    let day = at.get(..DAY_CHARS).unwrap_or("");
    if day >= LATE_ERA_FROM {
        Era::Late
    } else if day >= MID_ERA_FROM {
        Era::Mid
    } else {
        Era::Early
    }
}

fn era_at(entries: &[Entry], last: usize) -> Era {
    entries
        .get(last)
        .map_or(Era::Early, |entry| era_of(&entry.at))
}

fn block(group: Group, era: Option<Era>, data: &Collected, entries: &[Entry]) -> Block {
    let keep = |last: usize| era.is_none_or(|era| era_at(entries, last) == era);
    Block {
        survival: tally(
            data.observations
                .iter()
                .filter(|o| o.group == group && keep(o.last)),
        ),
        pointer: pointer_cell(
            data.pointers
                .iter()
                .filter(|p| p.group == group && keep(p.last)),
        ),
        reach: reach_tally(
            data.probes
                .iter()
                .filter(|p| p.group == group && keep(p.last)),
        ),
    }
}

pub fn section(group: Group, data: &Collected, entries: &[Entry]) -> Section {
    Section {
        overall: block(group, None, data, entries),
        eras: ERAS
            .iter()
            .map(|era| (*era, block(group, Some(*era), data, entries)))
            .collect(),
    }
}
