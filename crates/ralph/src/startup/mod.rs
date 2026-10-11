//! Prompt, summary, and conversation history display for the ask command.
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
pub use display::{display_ask_summary, display_conversation_history, display_prompt};
pub use types::{AskSummary, ConversationHistory, PromptDisplay};
