use crate::chat_memory::search::{tokenize, Token};
use crate::chat_memory::Entry;
use std::collections::{HashMap, HashSet};

const RARE_MAX_DF: usize = 3;
const RARE_MIN_LEN: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Ticket,
    Number,
    Identifier,
    Rare,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub word: String,
    pub class: Class,
}

pub type Frequency = HashMap<String, usize>;

pub fn document_frequency(entries: &[Entry]) -> Frequency {
    let mut frequency = Frequency::new();
    for entry in entries.iter().filter(|entry| !entry.is_corrupt()) {
        let words: HashSet<String> = tokenize(&entry.text)
            .into_iter()
            .map(|token| token.word)
            .collect();
        for word in words {
            *frequency.entry(word).or_insert(0) += 1;
        }
    }
    frequency
}

fn is_ticket(word: &str) -> bool {
    word.split_once('-').is_some_and(|(prefix, number)| {
        !prefix.is_empty()
            && prefix.bytes().all(|b| b.is_ascii_lowercase())
            && !number.is_empty()
            && number.bytes().all(|b| b.is_ascii_digit())
    })
}

pub fn classify(token: &Token, frequency: &Frequency) -> Option<Class> {
    let word = token.word.as_str();
    if is_ticket(word) {
        Some(Class::Ticket)
    } else if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
        Some(Class::Number)
    } else if !token.parts.is_empty() || word.contains('_') {
        Some(Class::Identifier)
    } else if word.chars().count() >= RARE_MIN_LEN
        && frequency.get(word).copied().unwrap_or(0) <= RARE_MAX_DF
    {
        Some(Class::Rare)
    } else {
        None
    }
}

pub fn key_tokens(text: &str, frequency: &Frequency) -> Vec<Key> {
    let mut seen = HashSet::new();
    tokenize(text)
        .iter()
        .filter_map(|token| {
            classify(token, frequency).map(|class| Key {
                word: token.word.clone(),
                class,
            })
        })
        .filter(|key| seen.insert(key.word.clone()))
        .collect()
}
pub fn text_words(text: &str) -> HashSet<String> {
    tokenize(text)
        .into_iter()
        .flat_map(|token| std::iter::once(token.word).chain(token.parts))
        .collect()
}
