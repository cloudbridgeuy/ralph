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
//! - Metadata and tool call extraction from stream events
//! - Output block accumulation for response text extraction
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

mod disk_files;
mod event_handler;
mod processor;
mod render_block;
mod tool_results;

pub use processor::{StreamProcessor, StreamProcessorBuilder};
pub use ralph_core::stream::blocks::{
    extract_response_text, GrepInvocationBuilder, OutputBlock, StreamProcessorResult, TextBlock,
    TodoItem, ToolInvocationBlock, ToolInvocationVariant, ToolResultBlock, ToolResultVariant,
    VerboseToolsConfig,
};

#[cfg(test)]
pub use ralph_core::stream::blocks::{extract_key_argument, truncate_string, KeyArgument};

#[cfg(test)]
mod tests;
