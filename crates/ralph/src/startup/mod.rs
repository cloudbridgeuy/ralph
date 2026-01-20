//! Startup information display for the run command.
//!
//! This module provides functions to display startup information when
//! `ralph run` begins, giving users immediate feedback about the session
//! being created and the work to be done.
//!
//! The display adapts based on terminal detection:
//! - Terminal: Uses box drawing characters and ANSI colors
//! - Piped: Uses plain ASCII with no ANSI codes

mod display;
mod formatters;
mod plain;
mod terminal;
mod types;

#[cfg(test)]
mod tests;

// Re-export public API
pub use display::{
    display_iteration_header, display_iteration_summary, display_prompt, display_run_summary,
    display_startup_info,
};
pub use formatters::format_duration;
pub use types::{
    IterationHeader, IterationSummary, PromptDisplay, RunSummary, StartupInfo, VERSION,
};
