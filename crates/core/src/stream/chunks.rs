//! Chunk parsing wrappers for stream events.

use crate::chunk::{parse_chunks, parse_chunks_with_heuristics, ParsedChunk};

use super::accumulation::accumulate_text;
use super::events::StreamEvent;

/// Parse accumulated text from stream events into typed chunks.
///
/// Takes accumulated text from assistant events (obtained via [`accumulate_text`] or
/// [`TextAccumulator`]) and parses it into typed chunks: prose, code, and diff.
///
/// This is a convenience function that applies the chunk parsing logic from
/// [`crate::chunk::parse_chunks`] to text extracted from stream events.
///
/// # Arguments
///
/// * `events` - A slice of stream events to process
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     parse_chunks_from_events, StreamEvent, AssistantEvent, AssistantMessage,
///     ContentBlock, ChunkType,
/// };
///
/// let events = vec![
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: Some("msg_01".to_string()),
///             content: vec![ContentBlock::Text {
///                 text: "Here's the code:\n\n```rust\nfn main() {}\n```\n\nDone!".to_string(),
///             }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// let chunks = parse_chunks_from_events(&events);
/// assert_eq!(chunks.len(), 3);
/// assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
/// assert!(matches!(chunks[1].chunk_type, ChunkType::Code { .. }));
/// assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
/// ```
pub fn parse_chunks_from_events(events: &[StreamEvent]) -> Vec<ParsedChunk> {
    let text = accumulate_text(events.iter());
    parse_chunks(&text)
}

/// Parse accumulated text from stream events into typed chunks with diff heuristics.
///
/// Similar to [`parse_chunks_from_events`], but also applies heuristic detection for
/// unfenced diff content (e.g., raw `diff --git` output not wrapped in code fences).
///
/// Use this function when processing output that may contain unfenced diffs,
/// such as when the LLM outputs raw git diff results directly.
///
/// # Arguments
///
/// * `events` - A slice of stream events to process
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     parse_chunks_from_events_with_heuristics, StreamEvent, AssistantEvent,
///     AssistantMessage, ContentBlock, ChunkType,
/// };
///
/// let events = vec![
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: Some("msg_01".to_string()),
///             content: vec![ContentBlock::Text {
///                 text: "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new".to_string(),
///             }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// let chunks = parse_chunks_from_events_with_heuristics(&events);
/// assert_eq!(chunks.len(), 1);
/// assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
/// ```
pub fn parse_chunks_from_events_with_heuristics(events: &[StreamEvent]) -> Vec<ParsedChunk> {
    let text = accumulate_text(events.iter());
    parse_chunks_with_heuristics(&text)
}

/// Parse text directly into typed chunks.
///
/// This is a re-export of [`crate::chunk::parse_chunks`] for convenience when working
/// with stream parsing. See that function for detailed documentation.
///
/// # Arguments
///
/// * `text` - The text to parse (typically accumulated from assistant events)
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
pub fn parse_text_into_chunks(text: &str) -> Vec<ParsedChunk> {
    parse_chunks(text)
}

/// Parse text directly into typed chunks with diff heuristics.
///
/// This is a re-export of [`crate::chunk::parse_chunks_with_heuristics`] for convenience
/// when working with stream parsing. See that function for detailed documentation.
///
/// # Arguments
///
/// * `text` - The text to parse (typically accumulated from assistant events)
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
pub fn parse_text_into_chunks_with_heuristics(text: &str) -> Vec<ParsedChunk> {
    parse_chunks_with_heuristics(text)
}
