use super::tokens::{key_tokens, Class, Frequency, Key};
use crate::chat_memory::{ids_with, Entry, Label, Role, TagState};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Tagged,
    TopUp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub id: usize,
    pub group: Group,
    pub keys: Vec<Key>,
}

fn tagged_ids(entries: &[Entry], state: &TagState) -> BTreeSet<usize> {
    let mut ids = ids_with(state, Label::Rule);
    ids.extend(ids_with(state, Label::Decision));
    ids.retain(|id| entries.get(*id).is_some_and(|entry| !entry.is_corrupt()));
    ids
}

fn names_a_thing(keys: &[Key]) -> bool {
    keys.iter()
        .any(|key| matches!(key.class, Class::Ticket | Class::Identifier))
}

pub fn pick_anchors(
    entries: &[Entry],
    state: &TagState,
    frequency: &Frequency,
    max: usize,
) -> Vec<Anchor> {
    let tagged = tagged_ids(entries, state);
    let first = tagged.iter().map(|id| Anchor {
        id: *id,
        group: Group::Tagged,
        keys: key_tokens(&entries[*id].text, frequency),
    });
    let top_up = entries
        .iter()
        .enumerate()
        .filter(|(id, entry)| {
            entry.role == Role::User && !entry.is_corrupt() && !tagged.contains(id)
        })
        .map(|(id, entry)| Anchor {
            id,
            group: Group::TopUp,
            keys: key_tokens(&entry.text, frequency),
        })
        .filter(|anchor| names_a_thing(&anchor.keys));
    first.chain(top_up).take(max).collect()
}
