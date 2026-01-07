//! Core types for parsed chunks.

use serde::{Deserialize, Serialize};

/// The type of a parsed chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ChunkType {
    /// Regular prose/markdown content.
    Prose,
    /// A fenced code block with optional language hint.
    Code {
        /// The language hint from the opening fence (e.g., "rust", "python").
        /// `None` if no language was specified.
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    /// A diff block (unified diff format).
    Diff,
}

/// A parsed chunk of LLM output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedChunk {
    /// The type of chunk (prose, code, or diff).
    #[serde(flatten)]
    pub chunk_type: ChunkType,
    /// The content of the chunk.
    pub content: String,
}

impl ParsedChunk {
    /// Create a prose chunk.
    pub fn prose(content: impl Into<String>) -> Self {
        Self {
            chunk_type: ChunkType::Prose,
            content: content.into(),
        }
    }

    /// Create a code chunk with optional language hint.
    pub fn code(content: impl Into<String>, language: Option<String>) -> Self {
        Self {
            chunk_type: ChunkType::Code { language },
            content: content.into(),
        }
    }

    /// Create a diff chunk.
    pub fn diff(content: impl Into<String>) -> Self {
        Self {
            chunk_type: ChunkType::Diff,
            content: content.into(),
        }
    }
}
