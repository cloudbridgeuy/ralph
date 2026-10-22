//! Subprocess invocation for LLM tools.
//!
//! This module provides functionality to invoke external LLM tools (like claude)
//! as subprocesses, capturing their output and exit codes. It follows the
//! Imperative Shell pattern, handling I/O operations that the functional core
//! cannot perform.
//!
//! [`invoke_subprocess_with_spinner_config`] runs a command with spinner display,
//! session info, timeout enforcement, and theme support.

use std::io;
use std::process::{Child, Command, Stdio};
use std::thread;

mod gate;
mod session;
mod spinner;
mod timeout;
mod types;

// Re-export public API
pub use gate::OutputGate;
pub use session::Session;
pub use spinner::{invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig};
pub use types::{StreamingSubprocessResult, SubprocessError};

/// Kill a subprocess and its entire process group.
///
/// Uses `libc::killpg` to send SIGKILL to the process group,
/// ensuring grandchild processes (e.g., `claude` spawned by `sh -c`)
/// are also terminated.
pub(crate) fn kill_process_group(child: &mut Child) {
    signal_group(child);
    // Wait to clean up zombie
    let _ = child.wait();
}

/// Sends SIGKILL to the child's process group. Call it only before the child
/// is reaped: until then the child (or its zombie) holds the pid, so the
/// group id cannot belong to another process.
pub(super) fn signal_group(child: &Child) {
    let pid = child.id() as libc::pid_t;
    // Safety: killpg sends a signal to a process group. The pid is valid
    // because we just obtained it from the child process.
    unsafe { libc::killpg(pid, libc::SIGKILL) };
}

/// Reports whether the child has exited, without reaping it. An error also
/// counts as exited, so callers stop waiting and reap.
pub(super) fn exited_unreaped(child: &Child) -> bool {
    // Safety: an all-zero siginfo_t is a valid out-parameter for waitid.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // Safety: waitid only writes to `info`. WNOWAIT leaves the child
    // waitable, so `Child::wait` still reaps it later.
    let found = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    found != 0 || info.si_signo != 0
}

/// Starts a background helper with null stdio and reaps it on a thread, so
/// it never stays a zombie while ralph runs.
pub(crate) fn spawn_detached(command: &mut Command) -> io::Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    thread::spawn(move || child.wait().ok());
    Ok(())
}
