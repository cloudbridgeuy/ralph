//! Subprocess invocation for LLM tools.
//!
//! This module provides functionality to invoke external LLM tools (like claude)
//! as subprocesses, capturing their output and exit codes. It follows the
//! Imperative Shell pattern, handling I/O operations that the functional core
//! cannot perform.
//!
//! # Variants
//!
//! - [`invoke_subprocess`] - Basic subprocess with line-by-line streaming
//! - [`invoke_subprocess_with_stream_processing`] - Enhanced subprocess with
//!   stream-json parsing, syntax highlighting, and metadata extraction

use crate::stream_processor::{StreamProcessor, StreamProcessorResult};
use std::io::{self, BufRead, BufReader, Write};
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

/// Result of a subprocess invocation with stream processing.
#[derive(Debug)]
pub struct StreamingSubprocessResult {
    /// The exit code from the subprocess.
    pub exit_code: i32,
    /// Captured stderr output.
    pub stderr: String,
    /// Processed stream result with chunks, metadata, and tool interactions.
    pub stream_result: StreamProcessorResult,
}

/// Invokes a command with stream-json output parsing and syntax highlighting.
///
/// This is an enhanced version of [`invoke_subprocess`] that:
/// 1. Parses Claude's `--output-format stream-json` output in real-time
/// 2. Applies syntax highlighting to code blocks
/// 3. Applies diff highlighting with delta fallback
/// 4. Extracts metadata (session ID, model, costs, usage)
/// 5. Correlates tool calls with their results
///
/// # Arguments
///
/// * `command` - The command string to execute (should produce stream-json output)
///
/// # Returns
///
/// Returns a [`StreamingSubprocessResult`] containing:
/// - Exit code
/// - Stderr output
/// - Parsed chunks with highlighting applied
/// - Metadata extracted from JSON events
/// - Tool interactions
///
/// # Behavior
///
/// - Spawns the command as a subprocess using the shell
/// - Inherits stdin so interactive prompts work
/// - Parses stdout as newline-delimited JSON
/// - Streams highlighted output to terminal as it arrives
/// - Captures full output for storage
///
/// # Example
///
/// ```no_run
/// use ralph::subprocess::invoke_subprocess_with_stream_processing;
///
/// let result = invoke_subprocess_with_stream_processing(
///     "claude --output-format stream-json -p 'hello'"
/// ).unwrap();
///
/// // Access parsed chunks
/// for chunk in &result.stream_result.chunks {
///     println!("Chunk: {:?}", chunk);
/// }
///
/// // Access metadata
/// if let Some(model) = &result.stream_result.metadata.model {
///     println!("Model: {}", model);
/// }
/// ```
pub fn invoke_subprocess_with_stream_processing(
    command: &str,
) -> Result<StreamingSubprocessResult, SubprocessError> {
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

    // Create stream processor
    let mut processor = StreamProcessor::new();

    // Capture stderr
    let mut stderr_captured = String::new();

    // Process stdout line by line through the stream processor
    for line in stdout_reader.lines() {
        let line = line.map_err(|e| {
            SubprocessError::OutputCaptureFailed(format!("Failed to read stdout: {}", e))
        })?;

        // Process line through stream processor
        if let Some(output) = processor.process_line(&line) {
            // Stream highlighted output to terminal
            print!("{}", output);
            // Flush to ensure immediate output
            let _ = io::stdout().flush();
        }
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

    // Finish stream processing
    let stream_result = processor.finish();

    Ok(StreamingSubprocessResult {
        exit_code,
        stderr: stderr_captured,
        stream_result,
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
        // Note: stderr may contain shell-init warnings in some environments
        // so we don't assert it's empty
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
        // Note: stderr may contain shell-init warnings in some environments
        // so we don't assert it's empty
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
