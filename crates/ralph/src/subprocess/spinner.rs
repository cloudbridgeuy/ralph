//! Subprocess invocation with spinner display and gap detection.

use super::timeout::try_wait_child;
use super::types::{
    StreamingSubprocessResult, SubprocessError, DEFAULT_GAP_THRESHOLD_MS, EXIT_CODE_INTERRUPTED,
    EXIT_CODE_KILLED,
};
use crate::highlight::ThemeConfig;
use crate::signal;
use crate::spinner::{Spinner, SpinnerContext, SpinnerSessionInfo};
use crate::stream_processor::{StreamProcessor, VerboseToolsConfig};
use std::io::IsTerminal;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Configuration for subprocess invocation with spinner.
///
/// Groups parameters to keep function signatures under 5 arguments.
#[derive(Debug, Clone)]
pub struct SpinnerSubprocessConfig {
    /// The command string to execute.
    pub command: String,
    /// Maximum duration in seconds before killing the subprocess.
    pub timeout_secs: u64,
    /// Configuration for syntax highlighting theme.
    pub theme_config: ThemeConfig,
    /// Accumulated time from previous iterations in this session.
    pub session_elapsed_ms: u64,
    /// Configuration for verbose tool output.
    pub verbose_tools: VerboseToolsConfig,
    /// Session metadata for spinner display (slug, iteration info).
    pub session_info: SpinnerSessionInfo,
}

/// Invokes a command with stream processing, theme configuration, and spinner display.
///
/// This extends [`super::invoke_subprocess_with_timeout`] with spinner support:
/// - Shows an animated spinner while waiting for LLM to respond
/// - Displays elapsed time updating every second
/// - Displays session name and iteration progress (if provided)
/// - Automatically stops spinner when first output arrives
/// - Only shows spinner when stdout is a terminal
///
/// # Arguments
///
/// * `config` - Configuration for the subprocess invocation
///
/// # Returns
///
/// Returns a [`StreamingSubprocessResult`] on success or `SubprocessError` on failure.
///
/// # Example
///
/// ```no_run
/// use ralph::subprocess::{invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig};
/// use ralph::highlight::ThemeConfig;
/// use ralph::stream_processor::VerboseToolsConfig;
/// use ralph::spinner::SpinnerSessionInfo;
///
/// let config = SpinnerSubprocessConfig {
///     command: "claude --output-format stream-json -p 'hello'".to_string(),
///     timeout_secs: 300,
///     theme_config: ThemeConfig::new().with_theme("Monokai Extended"),
///     session_elapsed_ms: 0,
///     verbose_tools: VerboseToolsConfig::new(),
///     session_info: SpinnerSessionInfo::new("brave-panda".to_string(), 1, 5),
/// };
/// let result = invoke_subprocess_with_spinner_config(&config);
/// ```
pub fn invoke_subprocess_with_spinner_config(
    config: &SpinnerSubprocessConfig,
) -> Result<StreamingSubprocessResult, SubprocessError> {
    // Spawn subprocess with stdout/stderr captured and stdin inherited
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&config.command)
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

    // Check if stdout is a terminal for highlighting and tool display
    let is_terminal = std::io::stdout().is_terminal();

    // Create stream processor with theme configuration and verbose tools
    let mut processor = StreamProcessor::with_verbose_tools(
        config.theme_config.clone(),
        is_terminal,
        is_terminal,
        config.verbose_tools.clone(),
    )?;

    // Create and start spinner with session context
    let mut spinner =
        Spinner::with_session_context(config.session_elapsed_ms, config.session_info.clone());
    spinner.start();

    // Track time for gap detection and spinner control
    let mut last_output_time = Instant::now();
    let mut spinner_active = true; // Spinner starts active
    let gap_threshold = Duration::from_millis(DEFAULT_GAP_THRESHOLD_MS);

    // Track whether we're in a tool invocation (to determine spinner context)
    let mut pending_tool_count: usize = 0;

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
    let timeout = Duration::from_secs(config.timeout_secs);

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
            // Stop spinner if active
            if spinner_active {
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
            // Stop spinner if active
            if spinner_active {
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
                timeout_secs: config.timeout_secs,
                partial_result: Box::new(StreamingSubprocessResult {
                    exit_code: EXIT_CODE_KILLED,
                    stderr: stderr_captured,
                    stream_result,
                }),
            });
        }

        // Check for interrupt signal (SIGINT/SIGTERM)
        if signal::is_interrupted() {
            // Stop spinner if active
            if spinner_active {
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
                    exit_code: EXIT_CODE_INTERRUPTED,
                    stderr: stderr_captured,
                    stream_result,
                }),
            });
        }

        // Try to receive a line with a short timeout
        match line_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line_result) => match line_result {
                Ok(line) => {
                    // Check for tool invocation markers in the JSON
                    // Tool invocations come in assistant events with tool_use content
                    if line.contains("\"type\":\"tool_use\"") {
                        pending_tool_count += 1;
                    }
                    // Tool results come in user events
                    if line.contains("\"type\":\"user\"")
                        && line.contains("\"type\":\"tool_result\"")
                    {
                        pending_tool_count = pending_tool_count.saturating_sub(1);
                    }

                    if let Some(output) = processor.process_line(&line) {
                        // Stop spinner on visible output
                        if spinner_active {
                            spinner.stop();
                            spinner_active = false;
                        }
                        print!("{}", output);
                        let _ = io::stdout().flush();
                        // Update last output time
                        last_output_time = Instant::now();
                    }
                }
                Err(e) => {
                    // Stop spinner before returning error
                    if spinner_active {
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

                // Gap detection: if no output for threshold duration, show spinner
                if !spinner_active && last_output_time.elapsed() >= gap_threshold {
                    // Determine spinner context based on state
                    let context = if pending_tool_count > 0 {
                        SpinnerContext::WaitingForTool
                    } else {
                        SpinnerContext::Thinking
                    };
                    spinner.start_with_context(context);
                    spinner_active = true;
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
