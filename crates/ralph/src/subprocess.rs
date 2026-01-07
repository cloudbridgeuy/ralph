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

use crate::highlight::{ThemeConfig, ThemeError};
use crate::signal;
use crate::spinner::Spinner;
use crate::stream_processor::{StreamProcessor, StreamProcessorResult};
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

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

    #[error("Subprocess timed out after {timeout_secs} seconds")]
    Timeout {
        /// Timeout duration in seconds
        timeout_secs: u64,
        /// Partial output captured before timeout
        partial_result: Box<StreamingSubprocessResult>,
    },

    #[error("Subprocess interrupted by SIGINT/SIGTERM")]
    Interrupted {
        /// Partial output captured before interrupt
        partial_result: Box<StreamingSubprocessResult>,
    },

    #[error("Invalid theme configuration: {0}")]
    ThemeError(#[from] ThemeError),
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

/// Invokes a command with stream-json output parsing, syntax highlighting, and timeout.
///
/// This extends [`invoke_subprocess_with_stream_processing`] with timeout support:
/// - If the subprocess exceeds `timeout_secs`, it is killed and a `Timeout` error is returned
/// - Partial output captured before timeout is included in the error
///
/// # Arguments
///
/// * `command` - The command string to execute (should produce stream-json output)
/// * `timeout_secs` - Maximum duration in seconds before killing the subprocess
///
/// # Returns
///
/// Returns a [`StreamingSubprocessResult`] on success or timeout.
/// On timeout, returns `SubprocessError::Timeout` with partial output.
///
/// # Example
///
/// ```no_run
/// use ralph::subprocess::invoke_subprocess_with_timeout;
///
/// // Run with 5 minute timeout
/// let result = invoke_subprocess_with_timeout(
///     "claude --output-format stream-json -p 'hello'",
///     300
/// );
///
/// match result {
///     Ok(r) => println!("Completed with exit code {}", r.exit_code),
///     Err(ralph::subprocess::SubprocessError::Timeout { timeout_secs, partial_result }) => {
///         eprintln!("Timed out after {} seconds", timeout_secs);
///         // Partial output is available in partial_result
///     }
///     Err(e) => eprintln!("Error: {}", e),
/// }
/// ```
pub fn invoke_subprocess_with_timeout(
    command: &str,
    timeout_secs: u64,
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

    // Create channel to receive stderr from background thread
    let (stderr_tx, stderr_rx) = mpsc::channel::<String>();

    // Spawn thread to capture stderr in background
    let stderr_thread = thread::spawn(move || {
        let mut captured = String::new();
        for line in stderr_reader.lines().map_while(Result::ok) {
            // Send to main thread for display
            let _ = stderr_tx.send(line.clone());
            captured.push_str(&line);
            captured.push('\n');
        }
        captured
    });

    // Track timeout
    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    // Process stdout line by line with timeout checking
    // Use a separate thread to read lines so we can check timeout
    let (line_tx, line_rx) = mpsc::channel::<io::Result<String>>();
    let stdout_thread = thread::spawn(move || {
        for line in stdout_reader.lines() {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    // Process lines with timeout
    loop {
        // Check if process has completed
        if let Some(status) = try_wait_child(&mut child)? {
            // Process completed, drain remaining output
            while let Ok(line_result) = line_rx.try_recv() {
                if let Ok(line) = line_result {
                    if let Some(output) = processor.process_line(&line) {
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
            }

            // Drain stderr
            while let Ok(line) = stderr_rx.try_recv() {
                eprintln!("{}", line);
            }

            // Wait for threads to finish
            let _ = stdout_thread.join();
            let stderr_captured = stderr_thread.join().unwrap_or_default();

            // Extract exit code
            let exit_code = status.code().ok_or(SubprocessError::Signaled)?;

            // Finish stream processing
            let stream_result = processor.finish();

            return Ok(StreamingSubprocessResult {
                exit_code,
                stderr: stderr_captured,
                stream_result,
            });
        }

        // Check timeout
        if start.elapsed() >= timeout {
            // Kill the subprocess
            let _ = child.kill();
            let _ = child.wait(); // Clean up zombie

            // Drain any remaining output that was received
            while let Ok(line_result) = line_rx.try_recv() {
                if let Ok(line) = line_result {
                    if let Some(output) = processor.process_line(&line) {
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
            }

            // Wait for threads
            let _ = stdout_thread.join();
            let stderr_captured = stderr_thread.join().unwrap_or_default();

            // Finish stream processing to get partial result
            let stream_result = processor.finish();

            return Err(SubprocessError::Timeout {
                timeout_secs,
                partial_result: Box::new(StreamingSubprocessResult {
                    exit_code: -1, // Indicate killed
                    stderr: stderr_captured,
                    stream_result,
                }),
            });
        }

        // Try to receive a line with a short timeout
        match line_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line_result) => match line_result {
                Ok(line) => {
                    if let Some(output) = processor.process_line(&line) {
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
                Err(e) => {
                    return Err(SubprocessError::OutputCaptureFailed(format!(
                        "Failed to read stdout: {}",
                        e
                    )));
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Check for stderr output
                while let Ok(line) = stderr_rx.try_recv() {
                    eprintln!("{}", line);
                }
                // Continue loop to check process status and timeout
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // stdout closed, wait for process to exit
                // Continue loop to check process status
            }
        }
    }
}

/// Invokes a command with stream-json output parsing, syntax highlighting, and theme configuration.
///
/// This extends [`invoke_subprocess_with_timeout`] with custom theme support:
/// - Allows specifying a custom syntax highlighting theme
/// - Supports disabling background colors
///
/// # Arguments
///
/// * `command` - The command string to execute (should produce stream-json output)
/// * `timeout_secs` - Maximum duration in seconds before killing the subprocess
/// * `theme_config` - Configuration for syntax highlighting theme
///
/// # Returns
///
/// Returns a [`StreamingSubprocessResult`] on success or `SubprocessError` on failure.
///
/// # Example
///
/// ```no_run
/// use ralph::subprocess::invoke_subprocess_with_theme;
/// use ralph::highlight::ThemeConfig;
///
/// let config = ThemeConfig::new().with_theme("Monokai Extended");
/// let result = invoke_subprocess_with_theme(
///     "claude --output-format stream-json -p 'hello'",
///     300,
///     config
/// );
/// ```
pub fn invoke_subprocess_with_theme(
    command: &str,
    timeout_secs: u64,
    theme_config: ThemeConfig,
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

    // Create stream processor with theme configuration
    let mut processor = StreamProcessor::with_theme_config(theme_config)?;

    // Create channel to receive stderr from background thread
    let (stderr_tx, stderr_rx) = mpsc::channel::<String>();

    // Spawn thread to capture stderr in background
    let stderr_thread = thread::spawn(move || {
        let mut captured = String::new();
        for line in stderr_reader.lines().map_while(Result::ok) {
            // Send to main thread for display
            let _ = stderr_tx.send(line.clone());
            captured.push_str(&line);
            captured.push('\n');
        }
        captured
    });

    // Track timeout
    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    // Process stdout line by line with timeout checking
    // Use a separate thread to read lines so we can check timeout
    let (line_tx, line_rx) = mpsc::channel::<io::Result<String>>();
    let stdout_thread = thread::spawn(move || {
        for line in stdout_reader.lines() {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    // Process lines with timeout
    loop {
        // Check if process has completed
        if let Some(status) = try_wait_child(&mut child)? {
            // Process completed, drain remaining output
            while let Ok(line_result) = line_rx.try_recv() {
                if let Ok(line) = line_result {
                    if let Some(output) = processor.process_line(&line) {
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
            }

            // Drain any remaining stderr
            while let Ok(line) = stderr_rx.try_recv() {
                eprintln!("{}", line);
            }

            // Wait for threads to finish
            let _ = stdout_thread.join();
            let stderr_captured = stderr_thread.join().unwrap_or_default();

            // Extract exit code
            let exit_code = status.code().ok_or(SubprocessError::Signaled)?;

            // Finish stream processing
            let stream_result = processor.finish();

            return Ok(StreamingSubprocessResult {
                exit_code,
                stderr: stderr_captured,
                stream_result,
            });
        }

        // Check timeout
        if start.elapsed() >= timeout {
            // Kill the subprocess
            let _ = child.kill();
            let _ = child.wait(); // Clean up zombie

            // Drain any remaining output that was received
            while let Ok(line_result) = line_rx.try_recv() {
                if let Ok(line) = line_result {
                    if let Some(output) = processor.process_line(&line) {
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
            }

            // Wait for threads
            let _ = stdout_thread.join();
            let stderr_captured = stderr_thread.join().unwrap_or_default();

            // Finish stream processing to get partial result
            let stream_result = processor.finish();

            return Err(SubprocessError::Timeout {
                timeout_secs,
                partial_result: Box::new(StreamingSubprocessResult {
                    exit_code: -1, // Indicate killed
                    stderr: stderr_captured,
                    stream_result,
                }),
            });
        }

        // Try to receive a line with a short timeout
        match line_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line_result) => match line_result {
                Ok(line) => {
                    if let Some(output) = processor.process_line(&line) {
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
                Err(e) => {
                    return Err(SubprocessError::OutputCaptureFailed(format!(
                        "Failed to read stdout: {}",
                        e
                    )));
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Check for stderr output
                while let Ok(line) = stderr_rx.try_recv() {
                    eprintln!("{}", line);
                }
                // Continue loop to check process status and timeout
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // stdout closed, wait for process to exit
                // Continue loop to check process status
            }
        }
    }
}

/// Helper to try waiting for child process without blocking.
fn try_wait_child(child: &mut Child) -> Result<Option<std::process::ExitStatus>, SubprocessError> {
    child.try_wait().map_err(|e| {
        SubprocessError::OutputCaptureFailed(format!("Failed to check subprocess status: {}", e))
    })
}

/// Invokes a command with stream processing, theme configuration, and spinner display.
///
/// This extends [`invoke_subprocess_with_theme`] with spinner support:
/// - Shows an animated spinner while waiting for LLM to respond
/// - Displays elapsed time updating every second
/// - Automatically stops spinner when first output arrives
/// - Only shows spinner when stdout is a terminal
///
/// # Arguments
///
/// * `command` - The command string to execute (should produce stream-json output)
/// * `timeout_secs` - Maximum duration in seconds before killing the subprocess
/// * `theme_config` - Configuration for syntax highlighting theme
/// * `session_elapsed_ms` - Accumulated time from previous iterations in this session
///
/// # Returns
///
/// Returns a [`StreamingSubprocessResult`] on success or `SubprocessError` on failure.
///
/// # Example
///
/// ```no_run
/// use ralph::subprocess::invoke_subprocess_with_spinner;
/// use ralph::highlight::ThemeConfig;
///
/// let config = ThemeConfig::new().with_theme("Monokai Extended");
/// let result = invoke_subprocess_with_spinner(
///     "claude --output-format stream-json -p 'hello'",
///     300,
///     config,
///     0, // No prior session time
/// );
/// ```
pub fn invoke_subprocess_with_spinner(
    command: &str,
    timeout_secs: u64,
    theme_config: ThemeConfig,
    session_elapsed_ms: u64,
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

    // Create stream processor with theme configuration
    let mut processor = StreamProcessor::with_theme_config(theme_config)?;

    // Create and start spinner
    let mut spinner = Spinner::with_session_elapsed(session_elapsed_ms);
    spinner.start();

    // Track if we've received first output (to stop spinner)
    let mut spinner_stopped = false;

    // Create channel to receive stderr from background thread
    let (stderr_tx, stderr_rx) = mpsc::channel::<String>();

    // Spawn thread to capture stderr in background
    let stderr_thread = thread::spawn(move || {
        let mut captured = String::new();
        for line in stderr_reader.lines().map_while(Result::ok) {
            // Send to main thread for display
            let _ = stderr_tx.send(line.clone());
            captured.push_str(&line);
            captured.push('\n');
        }
        captured
    });

    // Track timeout
    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    // Process stdout line by line with timeout checking
    // Use a separate thread to read lines so we can check timeout
    let (line_tx, line_rx) = mpsc::channel::<io::Result<String>>();
    let stdout_thread = thread::spawn(move || {
        for line in stdout_reader.lines() {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    // Process lines with timeout
    loop {
        // Check if process has completed
        if let Some(status) = try_wait_child(&mut child)? {
            // Stop spinner if not already stopped
            if !spinner_stopped {
                spinner.stop();
            }

            // Process completed, drain remaining output
            while let Ok(line_result) = line_rx.try_recv() {
                if let Ok(line) = line_result {
                    if let Some(output) = processor.process_line(&line) {
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
            }

            // Drain any remaining stderr
            while let Ok(line) = stderr_rx.try_recv() {
                eprintln!("{}", line);
            }

            // Wait for threads to finish
            let _ = stdout_thread.join();
            let stderr_captured = stderr_thread.join().unwrap_or_default();

            // Extract exit code
            let exit_code = status.code().ok_or(SubprocessError::Signaled)?;

            // Finish stream processing
            let stream_result = processor.finish();

            return Ok(StreamingSubprocessResult {
                exit_code,
                stderr: stderr_captured,
                stream_result,
            });
        }

        // Check timeout
        if start.elapsed() >= timeout {
            // Stop spinner before showing timeout message
            if !spinner_stopped {
                spinner.stop();
            }

            // Kill the subprocess
            let _ = child.kill();
            let _ = child.wait(); // Clean up zombie

            // Drain any remaining output that was received
            while let Ok(line_result) = line_rx.try_recv() {
                if let Ok(line) = line_result {
                    if let Some(output) = processor.process_line(&line) {
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
            }

            // Wait for threads
            let _ = stdout_thread.join();
            let stderr_captured = stderr_thread.join().unwrap_or_default();

            // Finish stream processing to get partial result
            let stream_result = processor.finish();

            return Err(SubprocessError::Timeout {
                timeout_secs,
                partial_result: Box::new(StreamingSubprocessResult {
                    exit_code: -1, // Indicate killed
                    stderr: stderr_captured,
                    stream_result,
                }),
            });
        }

        // Check for interrupt signal (SIGINT/SIGTERM)
        if signal::is_interrupted() {
            // Stop spinner
            if !spinner_stopped {
                spinner.stop();
            }

            // Kill the subprocess gracefully
            let _ = child.kill();
            let _ = child.wait(); // Clean up zombie

            // Drain any remaining output that was received
            while let Ok(line_result) = line_rx.try_recv() {
                if let Ok(line) = line_result {
                    if let Some(output) = processor.process_line(&line) {
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
            }

            // Wait for threads
            let _ = stdout_thread.join();
            let stderr_captured = stderr_thread.join().unwrap_or_default();

            // Finish stream processing to get partial result
            let stream_result = processor.finish();

            return Err(SubprocessError::Interrupted {
                partial_result: Box::new(StreamingSubprocessResult {
                    exit_code: -2, // Indicate interrupted
                    stderr: stderr_captured,
                    stream_result,
                }),
            });
        }

        // Try to receive a line with a short timeout
        match line_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line_result) => match line_result {
                Ok(line) => {
                    if let Some(output) = processor.process_line(&line) {
                        // Stop spinner on first visible output
                        if !spinner_stopped {
                            spinner.stop();
                            spinner_stopped = true;
                        }
                        print!("{}", output);
                        let _ = io::stdout().flush();
                    }
                }
                Err(e) => {
                    // Stop spinner before returning error
                    if !spinner_stopped {
                        spinner.stop();
                    }
                    return Err(SubprocessError::OutputCaptureFailed(format!(
                        "Failed to read stdout: {}",
                        e
                    )));
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Check for stderr output
                while let Ok(line) = stderr_rx.try_recv() {
                    eprintln!("{}", line);
                }
                // Continue loop to check process status and timeout
                // Spinner keeps running during this wait
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // stdout closed, wait for process to exit
                // Continue loop to check process status
            }
        }
    }
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

    // Timeout tests

    #[test]
    fn test_invoke_with_timeout_completes_quickly() {
        // Fast command should complete before timeout
        let result = invoke_subprocess_with_timeout("echo 'hello'", 10).unwrap();
        assert_eq!(result.exit_code, 0);
        // stream processor parses JSON, so plain text won't be captured meaningfully
    }

    #[test]
    fn test_invoke_with_timeout_times_out() {
        // Use a very short timeout (1 second) with a command that sleeps
        let result = invoke_subprocess_with_timeout("sleep 10", 1);
        match result {
            Err(SubprocessError::Timeout {
                timeout_secs,
                partial_result,
            }) => {
                assert_eq!(timeout_secs, 1);
                assert_eq!(partial_result.exit_code, -1); // Indicates killed
            }
            Ok(_) => panic!("Expected timeout error"),
            Err(e) => panic!("Expected timeout error, got: {}", e),
        }
    }

    #[test]
    fn test_invoke_with_timeout_captures_partial_output() {
        // Command that outputs something then sleeps
        // Using a very short timeout to catch it mid-output
        let result =
            invoke_subprocess_with_timeout("echo 'first'; sleep 10; echo 'never_reached'", 1);
        match result {
            Err(SubprocessError::Timeout { partial_result, .. }) => {
                // The partial result should exist, even if raw_text is empty
                // (because stream processor parses JSON, not plain text)
                assert!(partial_result.exit_code == -1);
            }
            Ok(_) => panic!("Expected timeout error"),
            Err(e) => panic!("Expected timeout error, got: {}", e),
        }
    }

    #[test]
    fn test_invoke_with_timeout_non_zero_exit() {
        // Command that fails should return the exit code, not timeout
        let result = invoke_subprocess_with_timeout("exit 42", 10).unwrap();
        assert_eq!(result.exit_code, 42);
    }

    #[test]
    fn test_timeout_error_display() {
        let partial = StreamingSubprocessResult {
            exit_code: -1,
            stderr: String::new(),
            stream_result: StreamProcessorResult::default(),
        };
        let err = SubprocessError::Timeout {
            timeout_secs: 300,
            partial_result: Box::new(partial),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("300 seconds"));
        assert!(msg.contains("timed out"));
    }
}
