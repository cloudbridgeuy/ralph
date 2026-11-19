use crate::chat_hold::HoldCompletion;
use crate::chat_view::DirEntry;
use std::ops::Range;

pub const MAX_SHOWN: usize = 200;
pub const MAX_READ: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRoot {
    Cwd,
    Root,
    Home,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathWord {
    pub span: Range<usize>,
    pub root: PathRoot,
    pub typed_dir: String,
    pub partial: String,
}

const HOME_VAR: &str = "$HOME";

impl PathWord {
    pub fn dir(&self) -> String {
        let typed = unescape(&self.typed_dir);
        match self.root {
            PathRoot::Home => typed
                .strip_prefix("~/")
                .or_else(|| typed.strip_prefix("$HOME/"))
                .unwrap_or_default()
                .to_string(),
            PathRoot::Cwd | PathRoot::Root => typed,
        }
    }
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(next) if c == '\\' && next.is_whitespace() => {}
            _ => out.push(c),
        }
    }
    out
}

fn escape(name: &str) -> String {
    name.chars().fold(String::new(), |mut out, c| {
        if c.is_whitespace() {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

fn word_start(before: &str) -> usize {
    let mut start = 0;
    let mut previous = None;
    for (index, c) in before.char_indices() {
        if c.is_whitespace() && previous != Some('\\') {
            start = index + c.len_utf8();
        }
        previous = Some(c);
    }
    start
}

fn root_of(word: &str) -> Option<PathRoot> {
    let head = word.split('/').next().unwrap_or_default();
    match head {
        "" if word.starts_with('/') => Some(PathRoot::Root),
        "~" | HOME_VAR => Some(PathRoot::Home),
        _ if head.starts_with('.') => Some(PathRoot::Cwd),
        _ => None,
    }
}

pub fn path_word(line: &str, cursor: usize) -> Option<PathWord> {
    let before = line.get(..cursor)?;
    let start = word_start(before);
    let word = &before[start..];
    let root = root_of(word)?;
    let first_token = before[..start].trim().is_empty();
    if root == PathRoot::Root && first_token && word.rfind('/') == Some(0) {
        return None;
    }
    let typed = match word {
        "~" | HOME_VAR | "." | ".." => format!("{word}/"),
        _ => word.to_string(),
    };
    let split = typed.rfind('/').map_or(0, |slash| slash + 1);
    Some(PathWord {
        span: start..cursor,
        root,
        typed_dir: typed[..split].to_string(),
        partial: unescape(&typed[split..]),
    })
}

pub fn path_completion(word: &PathWord, entries: &[DirEntry]) -> HoldCompletion {
    let shows_hidden = word.partial.starts_with('.');
    let mut values: Vec<String> = entries
        .iter()
        .filter(|entry| entry.name.starts_with(&word.partial))
        .filter(|entry| shows_hidden || !entry.name.starts_with('.'))
        .map(|entry| {
            let slash = if entry.is_dir { "/" } else { "" };
            format!("{}{}{slash}", word.typed_dir, escape(&entry.name))
        })
        .collect();
    values.sort();
    values.truncate(MAX_SHOWN);
    HoldCompletion {
        span: word.span.clone(),
        values,
    }
}

#[cfg(test)]
mod tests;
