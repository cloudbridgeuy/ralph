use crate::chat_help::Command;
use crate::chat_hold::{word_completion, HoldCompletion};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecCompletion {
    pub span: Range<usize>,
    pub values: Vec<SpecValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecValue {
    pub text: String,
    pub append_space: bool,
}

impl From<HoldCompletion> for SpecCompletion {
    fn from(found: HoldCompletion) -> Self {
        SpecCompletion {
            span: found.span,
            values: found
                .values
                .into_iter()
                .map(|text| SpecValue {
                    text,
                    append_space: false,
                })
                .collect(),
        }
    }
}

pub fn spec_completions(line: &str, cursor: usize) -> Option<SpecCompletion> {
    let typed = line.get(..cursor)?.trim_start();
    if !typed.starts_with('/') || typed.contains(char::is_whitespace) {
        return None;
    }
    let names = Command::ALL.map(|command| command.name().to_string());
    let found = word_completion(typed, cursor, names.into_iter().collect());
    let append_space = !line.get(cursor..)?.starts_with(char::is_whitespace);
    (!found.values.is_empty()).then(|| SpecCompletion {
        span: found.span,
        values: found
            .values
            .into_iter()
            .map(|text| SpecValue { text, append_space })
            .collect(),
    })
}

#[cfg(test)]
mod tests;
