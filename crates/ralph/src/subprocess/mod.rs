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

use std::process::Child;

mod spinner;
mod timeout;
mod types;

// Re-export public API
pub use spinner::{invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig};
pub use timeout::invoke_subprocess_with_timeout;
pub use types::{StreamingSubprocessResult, SubprocessError};

/// Kill a subprocess and its entire process group.
///
/// Uses `libc::killpg` to send SIGKILL to the process group,
/// ensuring grandchild processes (e.g., `claude` spawned by `sh -c`)
/// are also terminated.
fn kill_process_group(child: &mut Child) {
    let pid = child.id() as libc::pid_t;
    // Safety: killpg sends a signal to a process group. The pid is valid
    // because we just obtained it from the child process.
    unsafe { libc::killpg(pid, libc::SIGKILL) };
    // Wait to clean up zombie
    let _ = child.wait();
}
