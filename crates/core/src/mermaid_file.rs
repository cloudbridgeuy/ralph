use crate::ansi::{DIM, RESET};
use crate::file_link::path_line;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const DIAGRAMS_DIR: &str = "diagrams";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Miss {
    TooWide { needs: usize, has: usize },
    NoText,
}

pub fn diagram_name(source: &str, style: &[u8]) -> String {
    let digest = Sha256::new()
        .chain_update((source.len() as u64).to_le_bytes())
        .chain_update(source)
        .chain_update(style)
        .finalize();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{hex}.png")
}

pub fn miss_note(miss: Miss, saved: Option<&Path>) -> String {
    let why = match miss {
        Miss::TooWide { needs, has } => {
            format!("too big for this pane (needs {needs} columns, has {has})")
        }
        Miss::NoText => "no text view for this diagram".to_string(),
    };
    match saved {
        Some(path) => format!(
            "{DIM}mermaid: {why}; full size:{RESET}\n{}",
            path_line(path)
        ),
        None => format!("{DIM}mermaid: {why}; the full-size picture failed{RESET}\n"),
    }
}

#[cfg(test)]
mod tests;
