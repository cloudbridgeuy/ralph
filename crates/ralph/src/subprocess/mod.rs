//! Subprocess invocation for LLM tools.
//!
//! This module provides functionality to invoke external LLM tools (like claude)
//! as subprocesses, capturing their output and exit codes. It follows the
//! Imperative Shell pattern, handling I/O operations that the functional core
//! cannot perform.
//!
//! # Available Functions
//!
//! The module provides subprocess invocation variants:
//!
//! 1. [`invoke_subprocess_with_timeout`] - Stream processing with timeout enforcement
//! 2. [`invoke_subprocess_with_spinner_config`] - Adds spinner display with session info and theme support

mod spinner;
mod timeout;
mod types;

// Re-export public API
pub use spinner::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, SpinnerSubprocessOutcome,
};
pub use timeout::invoke_subprocess_with_timeout;
pub use types::{StreamingSubprocessResult, SubprocessError};
