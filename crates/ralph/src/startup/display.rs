//! Public display functions with terminal detection.

use std::io::IsTerminal;

use super::plain;
use super::terminal;
use super::types::{IterationHeader, IterationSummary, RunSummary, StartupInfo};

/// Display startup information to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses box drawing characters and colors
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_startup_info(info: &StartupInfo) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        terminal::display_startup_terminal(info);
    } else {
        plain::display_startup_plain(info);
    }
}

/// Display iteration header to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses box drawing characters and colors
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_iteration_header(header: &IterationHeader) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        terminal::display_iteration_header_terminal(header);
    } else {
        plain::display_iteration_header_plain(header);
    }
}

/// Display iteration summary to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses dimmed colors for a subtle summary appearance
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_iteration_summary(summary: &IterationSummary) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        terminal::display_iteration_summary_terminal(summary);
    } else {
        plain::display_iteration_summary_plain(summary);
    }
}

/// Display final run summary to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses box drawing characters and colors with clear boundaries
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_run_summary(summary: &RunSummary) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        terminal::display_run_summary_terminal(summary);
    } else {
        plain::display_run_summary_plain(summary);
    }
}
