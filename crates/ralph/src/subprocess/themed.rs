//! Subprocess invocation with custom theme configuration.

use super::timeout::try_wait_child;
use super::types::{StreamingSubprocessResult, SubprocessError};
use crate::highlight::ThemeConfig;
use crate::stream_processor::StreamProcessor;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Invokes a command with stream-json output parsing, syntax highlighting, and theme configuration.
///
/// This extends [`super::invoke_subprocess_with_timeout`] with custom theme support:
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
