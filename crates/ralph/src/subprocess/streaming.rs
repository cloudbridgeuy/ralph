//! Subprocess invocation with stream-json parsing and syntax highlighting.

use super::types::{StreamingSubprocessResult, SubprocessError};
use crate::stream_processor::StreamProcessor;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};

/// Invokes a command with stream-json output parsing and syntax highlighting.
///
/// This is an enhanced version of [`super::invoke_subprocess`] that:
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
