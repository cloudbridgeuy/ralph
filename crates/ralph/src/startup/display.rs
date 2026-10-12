//! Public display functions with terminal detection.

use std::io::IsTerminal;

use super::plain;
use super::terminal;
use super::types::{AskSummary, PromptDisplay};

/// Display the prompt before iterations begin.
///
/// Shows the prompt that will be passed to the LLM with a visual header.
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses markdown rendering with ANSI formatting
/// - Piped: Uses plain text without ANSI codes
pub fn display_prompt(prompt: &PromptDisplay) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        terminal::display_prompt_terminal(prompt);
    } else {
        plain::display_prompt_plain(prompt);
    }
}

/// Display ask command summary to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses colors and formatting
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_ask_summary(summary: &AskSummary) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        terminal::display_ask_summary_terminal(summary);
    } else {
        plain::display_ask_summary_plain(summary);
    }
}
