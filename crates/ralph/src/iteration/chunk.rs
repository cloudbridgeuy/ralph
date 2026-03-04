//! Chunk struct for typed output segments from the LLM.

use serde::{Deserialize, Serialize};

/// A chunk of output from the LLM.
///
/// Chunks represent typed segments of LLM output: prose (markdown), code
/// (fenced code blocks with optional language), and diff (unified diff format).
/// These are stored in iteration logs for replay with proper syntax highlighting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    /// Type of chunk: "prose", "code", or "diff"
    #[serde(rename = "type")]
    pub chunk_type: String,
    /// The actual content
    pub content: String,
    /// Optional language hint (for code chunks)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

impl Chunk {
    /// Create a prose chunk (plain text/markdown).
    pub fn prose(content: String) -> Self {
        Self {
            chunk_type: "prose".to_string(),
            content,
            language: None,
        }
    }

    /// Create a code chunk with optional language hint.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::iteration::Chunk;
    ///
    /// let chunk = Chunk::code("fn main() {}".to_string(), Some("rust".to_string()));
    /// assert_eq!(chunk.chunk_type, "code");
    /// assert_eq!(chunk.language, Some("rust".to_string()));
    /// ```
    pub fn code(content: String, language: Option<String>) -> Self {
        Self {
            chunk_type: "code".to_string(),
            content,
            language,
        }
    }

    /// Create a diff chunk.
    pub fn diff(content: String) -> Self {
        Self {
            chunk_type: "diff".to_string(),
            content,
            language: None,
        }
    }

    /// Convert from a `ParsedChunk` from the core library.
    ///
    /// This method provides the bridge between the functional core chunk types
    /// and the iteration log storage format.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::iteration::Chunk;
    /// use ralph_core::chunk::{ParsedChunk, ChunkType};
    ///
    /// let parsed = ParsedChunk::code("fn main() {}", Some("rust".to_string()));
    /// let chunk = Chunk::from_parsed_chunk(&parsed);
    ///
    /// assert_eq!(chunk.chunk_type, "code");
    /// assert_eq!(chunk.content, "fn main() {}");
    /// assert_eq!(chunk.language, Some("rust".to_string()));
    /// ```
    pub fn from_parsed_chunk(parsed: &ralph_core::chunk::ParsedChunk) -> Self {
        use ralph_core::chunk::ChunkType;

        match &parsed.chunk_type {
            ChunkType::Prose => Self::prose(parsed.content.clone()),
            ChunkType::Code { language } => Self::code(parsed.content.clone(), language.clone()),
            ChunkType::Diff => Self::diff(parsed.content.clone()),
            ChunkType::Directive { .. } => Self::prose(parsed.content.clone()),
        }
    }

    /// Convert multiple `ParsedChunk`s to `Chunk`s.
    ///
    /// This is a convenience method for batch conversion, preserving order.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::iteration::Chunk;
    /// use ralph_core::chunk::ParsedChunk;
    ///
    /// let parsed_chunks = vec![
    ///     ParsedChunk::prose("intro"),
    ///     ParsedChunk::code("fn main() {}", Some("rust".to_string())),
    /// ];
    ///
    /// let chunks = Chunk::from_parsed_chunks(&parsed_chunks);
    /// assert_eq!(chunks.len(), 2);
    /// assert_eq!(chunks[0].chunk_type, "prose");
    /// assert_eq!(chunks[1].chunk_type, "code");
    /// ```
    pub fn from_parsed_chunks(parsed: &[ralph_core::chunk::ParsedChunk]) -> Vec<Self> {
        parsed.iter().map(Chunk::from_parsed_chunk).collect()
    }
}
