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

impl PathWord {
    pub fn dir(&self) -> String {
        String::new()
    }
}

pub fn path_word(_line: &str, _cursor: usize) -> Option<PathWord> {
    None
}

pub fn path_completion(word: &PathWord, _entries: &[DirEntry]) -> HoldCompletion {
    HoldCompletion {
        span: word.span.clone(),
        values: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
