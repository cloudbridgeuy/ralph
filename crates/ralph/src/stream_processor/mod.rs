//! Streaming output processor for LLM subprocess output (Imperative Shell).
//!
//! This module provides real-time parsing and highlighting of Claude's
//! `--output-format stream-json` output. It parses JSON events line by line,
//! extracts text content, applies syntax highlighting to code blocks, and
//! outputs to the terminal while capturing data for storage.
//!
//! # Features
//!
//! - Real-time JSON parsing of stream-json events
//! - Syntax highlighting for code blocks using syntect
//! - Diff highlighting with delta fallback chain
//! - Terminal detection for automatic color support
//! - Metadata and tool call extraction for iteration logs
//! - Output block accumulation for replay serialization
//!
//! # Example
//!
//! ```no_run
//! use ralph::stream_processor::StreamProcessor;
//!
//! let mut processor = StreamProcessor::new();
//! processor.process_line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}"#);
//! let result = processor.finish();
//! ```

mod block_builders;
mod event_handler;
mod output_block;
mod processor;
mod result_handlers;
mod tool_display;
mod tool_results;
mod types;
mod utils;

// Re-export public API
pub use output_block::{
    GrepInvocationBuilder, OutputBlock, TextBlock, TodoItem, ToolInvocationBlock,
    ToolInvocationVariant, ToolResultBlock, ToolResultVariant,
};
pub use processor::{StreamProcessor, StreamProcessorBuilder};
pub use types::{StreamProcessorResult, VerboseToolsConfig};

// Re-export for tests
#[cfg(test)]
pub use types::KeyArgument;
#[cfg(test)]
pub use utils::{extract_key_argument, truncate_string};

#[cfg(test)]
mod test_helpers;
#[cfg(test)]
mod tests;
