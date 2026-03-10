//! Startup information display for strategy execution.
//!
//! This module provides functions to display startup information when
//! a strategy begins execution, giving users immediate feedback about the session
//! being created and the work to be done.
//!
//! The display adapts based on terminal detection:
//! - Terminal: Uses box drawing characters and ANSI colors
//! - Piped: Uses plain ASCII with no ANSI codes

mod display;
pub(crate) mod formatters;
mod plain;
mod terminal;
mod types;

#[cfg(test)]
mod tests;

// Re-export public API
pub use display::{
    display_ask_summary, display_conversation_history, display_iteration_header,
    display_iteration_summary, display_prompt, display_run_summary, display_startup_info,
};
pub use formatters::format_duration;
pub use types::{
    AskSummary, AttachedFile, ConversationHistory, ConversationTurn, IterationHeader,
    IterationSummary, PromptDisplay, RunSummary, StartupInfo, VERSION,
};
