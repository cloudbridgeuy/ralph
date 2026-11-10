//! Subprocess invocation for LLM tools.
//!
//! This module provides functionality to invoke external LLM tools (like claude)
//! as subprocesses, capturing their output and exit codes. It follows the
//! Imperative Shell pattern, handling I/O operations that the functional core
//! cannot perform.
//!
//! [`invoke_subprocess_with_spinner_config`] runs a command with spinner display,
//! session info, timeout enforcement, and theme support.

mod gate;
mod session;
mod spinner;
mod timeout;
mod types;

// Re-export public API
pub use gate::OutputGate;
pub(crate) use ralph_server::subprocess::{
    exited_unreaped, kill_process_group, signal_group, spawn_detached, timed_output, TimedOutput,
};
pub use session::Session;
pub use spinner::{
    invoke_subprocess_silent, invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig,
};
pub use types::{recorded, StreamingSubprocessResult, SubprocessError};
