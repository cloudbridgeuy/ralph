//! Subprocess invocation for LLM tools.
//!
//! This module provides functionality to invoke external LLM tools (like claude)
//! as subprocesses, capturing their output and exit codes. It follows the
//! Imperative Shell pattern, handling I/O operations that the functional core
//! cannot perform.
//!
//! # Progressive Composition
//!
//! The module provides multiple invocation variants with increasing functionality:
//!
//! 1. [`invoke_subprocess`] - Basic subprocess with line-by-line streaming
//! 2. [`invoke_subprocess_with_stream_processing`] - Adds JSON parsing and highlighting
//! 3. [`invoke_subprocess_with_timeout`] - Adds timeout enforcement
//! 4. [`invoke_subprocess_with_theme`] - Adds custom theme configuration
//! 5. [`invoke_subprocess_with_spinner_config`] - Adds spinner display with session info

mod basic;
mod spinner;
mod streaming;
mod themed;
mod timeout;
mod types;

// Re-export public API
pub use basic::invoke_subprocess;
pub use spinner::{invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig};
pub use streaming::invoke_subprocess_with_stream_processing;
pub use themed::invoke_subprocess_with_theme;
pub use timeout::invoke_subprocess_with_timeout;
pub use types::{StreamingSubprocessResult, SubprocessError, SubprocessResult};
