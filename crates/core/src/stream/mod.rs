//! Stream-JSON event types for Claude CLI output parsing.
//!
//! This module defines Rust types that correspond to Claude CLI's `--output-format stream-json`
//! output. The events are newline-delimited JSON (NDJSON) that describe the LLM conversation
//! including system initialization, assistant responses, tool interactions, and final results.
//!
//! # Event Types
//!
//! - [`StreamEvent::System`] - Session initialization with metadata
//! - [`StreamEvent::Assistant`] - LLM responses containing text and/or tool calls
//! - [`StreamEvent::User`] - Tool execution results
//! - [`StreamEvent::Result`] - Final costs, usage, and duration
//!
//! # Example
//!
//! ```
//! use ralph_core::stream::StreamEvent;
//!
//! let json = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-opus-4-5"}"#;
//! let event: StreamEvent = serde_json::from_str(json).unwrap();
//!
//! match event {
//!     StreamEvent::System(sys) => {
//!         assert_eq!(sys.session_id, Some("abc-123".to_string()));
//!     }
//!     _ => panic!("Expected system event"),
//! }
//! ```

mod accumulation;
mod chunks;
mod costs;
mod events;
mod extraction;
mod metadata;
mod parsing;
mod tools;

#[cfg(test)]
mod tests;

// Re-export all public types and functions
pub use accumulation::{accumulate_text, AccumulatedMessage, TextAccumulator};
pub use chunks::{
    parse_chunks_from_events, parse_chunks_from_events_with_heuristics, parse_text_into_chunks,
    parse_text_into_chunks_with_heuristics,
};
pub use costs::{extract_costs_from_events, extract_costs_from_events_or_default, IterationCosts};
pub use events::{
    AssistantEvent, AssistantMessage, ContentBlock, ResultEvent, StreamEvent, SystemEvent, Tool,
    ToolResult, Usage, UserEvent, UserMessage,
};
pub use extraction::{
    extract_text_from_events, extract_tool_invocations_from_events, ToolInvocation,
};
pub use metadata::{
    extract_metadata_from_events, extract_metadata_from_events_or_default, IterationMetadata,
};
pub use parsing::{parse_stream_line, parse_stream_output, ParsedLine, StreamParser};
pub use tools::{correlate_tool_interactions, ToolCorrelator, ToolInteraction};

// Re-export chunk types for convenience
pub use crate::chunk::{parse_chunks, parse_chunks_with_heuristics, ChunkType, ParsedChunk};
