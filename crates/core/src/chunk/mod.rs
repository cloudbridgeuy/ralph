//! Chunk parsing for LLM output (Functional Core).
//!
//! This module provides pure functions for parsing accumulated text from
//! Claude's assistant events into typed chunks: prose (markdown), code
//! (fenced code blocks), and diff (unified diff format).
//!
//! # Chunk Types
//!
//! - **Prose**: Regular markdown text between code/diff blocks
//! - **Code**: Fenced code blocks (```) with optional language hint
//! - **Diff**: Unified diff format (```diff fence or heuristic detection)
//!
//! # Batch vs Streaming
//!
//! This module provides two approaches to chunk parsing:
//!
//! - **Batch**: Use [`parse_chunks`] when you have the complete text available
//! - **Streaming**: Use [`StreamingChunkBuffer`] to process text incrementally
//!
//! The streaming approach is useful when processing LLM output as it arrives,
//! allowing code blocks to be buffered until complete while prose streams eagerly.
//!
//! # Example (Batch)
//!
//! ```
//! use ralph_core::chunk::{parse_chunks, ChunkType};
//!
//! let text = "I'll implement:\n\n```rust\nfn hello() {}\n```\n\nDone.";
//! let chunks = parse_chunks(text);
//! assert_eq!(chunks.len(), 3);
//! assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
//! assert!(matches!(chunks[1].chunk_type, ChunkType::Code { .. }));
//! assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
//! ```
//!
//! # Example (Streaming)
//!
//! ```
//! use ralph_core::chunk::{StreamingChunkBuffer, ChunkType};
//!
//! let mut buffer = StreamingChunkBuffer::new();
//!
//! // Process lines as they arrive - prose emits eagerly
//! let chunks = buffer.process_line("Here's some code:");
//! assert_eq!(chunks.len(), 1);
//!
//! // Opening fence starts code block buffering
//! let chunks = buffer.process_line("```rust");
//! assert!(chunks.is_empty());
//!
//! // Code content is buffered
//! let chunks = buffer.process_line("fn main() {}");
//! assert!(chunks.is_empty());
//!
//! // Closing fence emits the complete code block
//! let chunks = buffer.process_line("```");
//! assert_eq!(chunks.len(), 1);
//!
//! // Get any remaining content
//! let final_chunks = buffer.finish();
//! ```

mod batch;
mod fence;
mod heuristics;
mod streaming;
mod types;

#[cfg(test)]
mod tests;

// Re-export public types
pub use batch::parse_chunks;
pub use heuristics::{is_unfenced_diff, parse_chunks_with_heuristics};
pub use streaming::{split_lines_preserve_trailing, StreamingChunkBuffer};
pub use types::{ChunkType, ParsedChunk};
