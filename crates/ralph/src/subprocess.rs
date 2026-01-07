//! Subprocess invocation for LLM tools.
//!
//! This module provides functionality to invoke external LLM tools (like claude)
//! as subprocesses, capturing their output and exit codes. It follows the
//! Imperative Shell pattern, handling I/O operations that the functional core
//! cannot perform.

#![allow(dead_code)] // Module not yet used by CLI commands

use std::io::{self, BufRead, BufReader};
use std::process::{Command, Stdio};

/// Result of a subprocess invocation.
#[derive(Debug, Clone)]
pub struct SubprocessResult {
    /// The exit code from the subprocess.
    pub exit_code: i32,
    /// Captured stdout output.
    pub stdout: String,
    /// Captured stderr output.
    pub stderr: String,
}

/// Error type for subprocess operations.
#[derive(Debug, thiserror::Error)]
pub enum SubprocessError {
    #[error("Failed to spawn subprocess: {0}")]
    SpawnFailed(#[from] io::Error),

    #[error("Subprocess terminated by signal")]
    Signaled,

    #[error("Failed to capture output: {0}")]
    OutputCaptureFailed(String),
}

/// Invokes a command as a subprocess, streaming output to the terminal in real-time
/// while also capturing it for storage.
///
/// # Arguments
///
/// * `command` - The command string to execute (e.g., "claude --output-format stream-json -p 'prompt'")
///
/// # Returns
///
/// Returns a `SubprocessResult` containing the exit code and captured output.
///
/// # Behavior
///
/// - Spawns the command as a subprocess using the shell
/// - Inherits stdin so interactive prompts work
/// - Streams stdout to the terminal as it arrives
/// - Captures full stdout and stderr for storage
/// - Returns exit code after process completion
///
/// # Example
///
/// ```no_run
/// use ralph::subprocess::invoke_subprocess;
///
/// let result = invoke_subprocess("echo 'Hello, world!'").unwrap();
/// assert_eq!(result.exit_code, 0);
/// assert!(result.stdout.contains("Hello, world!"));
/// ```
pub fn invoke_subprocess(command: &str) -> Result<SubprocessResult, SubprocessError> {
    // Spawn subprocess with stdout/stderr captured and stdin inherited
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::inherit()) // Inherit stdin for interactive prompts
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Get handles to stdout and stderr
    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let stderr = child.stderr.take().expect("Failed to capture stderr");

    // Create buffered readers
    let stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    // Capture output while streaming to terminal
    let mut stdout_captured = String::new();
    let mut stderr_captured = String::new();

    // Read stdout line by line, streaming to terminal and capturing
    for line in stdout_reader.lines() {
        let line = line.map_err(|e| {
            SubprocessError::OutputCaptureFailed(format!("Failed to read stdout: {}", e))
        })?;

        // Stream to terminal
        println!("{}", line);

        // Capture for storage
        stdout_captured.push_str(&line);
        stdout_captured.push('\n');
    }

    // Read stderr (after stdout completes)
    for line in stderr_reader.lines() {
        let line = line.map_err(|e| {
            SubprocessError::OutputCaptureFailed(format!("Failed to read stderr: {}", e))
        })?;

        // Stream to terminal (stderr)
        eprintln!("{}", line);

        // Capture for storage
        stderr_captured.push_str(&line);
        stderr_captured.push('\n');
    }

    // Wait for process completion
    let status = child.wait().map_err(|e| {
        SubprocessError::OutputCaptureFailed(format!("Failed to wait for subprocess: {}", e))
    })?;

    // Extract exit code
    let exit_code = status.code().ok_or(SubprocessError::Signaled)?;

    Ok(SubprocessResult {
        exit_code,
        stdout: stdout_captured,
        stderr: stderr_captured,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invoke_subprocess_success() {
        let result = invoke_subprocess("echo 'Hello, world!'").unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("Hello, world!"));
        assert!(result.stderr.is_empty());
    }

    #[test]
    fn test_invoke_subprocess_failure() {
        let result = invoke_subprocess("exit 42").unwrap();
        assert_eq!(result.exit_code, 42);
    }

    #[test]
    fn test_invoke_subprocess_stderr() {
        let result = invoke_subprocess("echo 'error message' >&2").unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stderr.contains("error message"));
    }

    #[test]
    fn test_invoke_subprocess_multiline_output() {
        let result = invoke_subprocess("echo 'line1'; echo 'line2'; echo 'line3'").unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("line1"));
        assert!(result.stdout.contains("line2"));
        assert!(result.stdout.contains("line3"));
    }

    #[test]
    fn test_invoke_subprocess_empty_output() {
        let result = invoke_subprocess("true").unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
    }

    #[test]
    fn test_invoke_subprocess_command_not_found() {
        let result = invoke_subprocess("nonexistent_command_12345");
        // Command will fail with non-zero exit code (command not found)
        assert!(result.is_ok()); // sh itself succeeds
        let result = result.unwrap();
        assert_ne!(result.exit_code, 0); // But the command inside fails
    }
}
