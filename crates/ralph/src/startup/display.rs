//! Public display functions with terminal detection.

use std::io::IsTerminal;

use super::plain;
use super::terminal;
use super::types::{AskSummary, PromptDisplay};
use crate::ui_theme::UiColors;

fn colored_stdout() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

/// Display the prompt before iterations begin.
///
/// Shows the prompt that will be passed to the LLM with a visual header.
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal, and `NO_COLOR` unset: markdown rendering with ANSI formatting
/// - Piped, or `NO_COLOR` set: plain text without ANSI codes
pub fn display_prompt(prompt: &PromptDisplay, ui: &UiColors) {
    if colored_stdout() {
        terminal::display_prompt_terminal(prompt, ui);
    } else {
        plain::display_prompt_plain(prompt);
    }
}

/// Display ask command summary to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal, and `NO_COLOR` unset: colors and formatting
/// - Piped, or `NO_COLOR` set: plain ASCII with no ANSI codes
pub fn display_ask_summary(summary: &AskSummary, ui: &UiColors) {
    if colored_stdout() {
        terminal::display_ask_summary_terminal(summary, ui);
    } else {
        plain::display_ask_summary_plain(summary);
    }
}
