use super::ancestors::Ancestor;
use super::anchors::{Anchor, Group};
use super::tokens::{text_words, Class};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub group: Group,
    pub depth: u32,
    pub class: Class,
    pub kept: bool,
    pub last: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pointer {
    pub group: Group,
    pub found: bool,
    pub last: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cell {
    pub kept: usize,
    pub total: usize,
}

impl Cell {
    pub fn percent(self) -> Option<usize> {
        (self.total > 0).then(|| (self.kept * 100 + self.total / 2) / self.total)
    }

    pub fn add(&mut self, kept: bool) {
        self.total += 1;
        self.kept += usize::from(kept);
    }
}

pub type Table = BTreeMap<(u32, Class), Cell>;

pub fn observe(anchor: &Anchor, ancestors: &[Ancestor]) -> Vec<Observation> {
    ancestors
        .iter()
        .flat_map(|ancestor| {
            let words = text_words(ancestor.summary);
            anchor.keys.iter().map(move |key| Observation {
                group: anchor.group,
                depth: ancestor.depth,
                class: key.class,
                kept: words.contains(&key.word),
                last: ancestor.node.hi - 1,
            })
        })
        .collect()
}

pub fn has_pointer(summary: &str, id: usize) -> bool {
    let needle = format!("#{id}");
    summary
        .match_indices(&needle)
        .any(|(at, _)| !summary[at + needle.len()..].starts_with(|c: char| c.is_ascii_digit()))
}

pub fn pointer_of(anchor: &Anchor, ancestors: &[Ancestor]) -> Option<Pointer> {
    let last = ancestors.last()?.node.hi - 1;
    Some(Pointer {
        group: anchor.group,
        found: ancestors
            .iter()
            .any(|ancestor| has_pointer(ancestor.summary, anchor.id)),
        last,
    })
}

pub fn tally<'a>(observations: impl IntoIterator<Item = &'a Observation>) -> Table {
    let mut table = Table::new();
    for obs in observations {
        table
            .entry((obs.depth, obs.class))
            .or_default()
            .add(obs.kept);
    }
    table
}

pub fn pointer_cell<'a>(pointers: impl IntoIterator<Item = &'a Pointer>) -> Cell {
    let mut cell = Cell::default();
    for pointer in pointers {
        cell.add(pointer.found);
    }
    cell
}
