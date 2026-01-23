//! Iteration log writing (Imperative Shell).
//!
//! This module handles writing iteration logs to disk after each LLM invocation.
//! Each iteration is stored as iteration-N.toml in the session directory with:
//! - Sequence number and timestamps
//! - Exit code from the subprocess
//! - Pending story counts before and after the iteration
//! - Metadata from JSON streaming output (session_id, model, cost, usage)
//! - Output chunks (prose, code, diff blocks)
//!
//! The metadata section is populated from Claude's `--output-format stream-json`
//! events, extracting session information from system init events and cost/usage
//! data from result events.

mod chunk;
mod error;
mod log;
mod metadata;
mod tool_call;
mod writer;

#[cfg(test)]
mod tests;

// Re-export public API
pub use chunk::Chunk;
pub use error::IterationError;
pub use log::IterationLog;
pub use metadata::LogMetadata;
pub use tool_call::LogToolCall;
pub use writer::{count_iterations, write_iteration_log};

/// Maximum size in bytes for tool results before truncation.
/// Results larger than this will be truncated with an indicator.
pub(crate) const MAX_RESULT_SIZE: usize = 10_000;
