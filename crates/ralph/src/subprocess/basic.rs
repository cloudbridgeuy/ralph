//! Basic subprocess invocation with line-by-line streaming.

use super::types::{SubprocessError, SubprocessResult};
use std::io::BufRead;
use std::io::BufReader;
use std::process::{Command, Stdio};

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
    // These should always be Some since we configured Stdio::piped()
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SubprocessError::OutputCaptureFailed("stdout not captured".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| SubprocessError::OutputCaptureFailed("stderr not captured".to_string()))?;

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
