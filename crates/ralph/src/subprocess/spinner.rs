//! Subprocess invocation with spinner display and gap detection.
//!
//! This module handles subprocess invocation with interactive features:
//! - Animated spinner during wait periods
//! - Gap detection to show spinner when LLM is thinking
//! - Keyboard input polling for interactive controls
//!
//! # Keyboard Controls
//!
//! When running in an interactive terminal, the subprocess loop polls for
//! keyboard input during wait periods. This enables interactive controls
//! like soft stop, hard stop, and pause (implemented in caller layers).
//!
//! # Raw Mode Lifecycle
//!
//! Raw terminal mode is carefully managed to avoid corrupting subprocess output:
//!
//! 1. **When enabled**: During spinner display periods only
//! 2. **When disabled**: Before any subprocess stdout/stderr is written
//! 3. **Panic safety**: [`KeyboardState`] uses [`RawModeGuard`] (RAII pattern)
//!    from the [`crate::keyboard`] module, ensuring cleanup on all exit paths
//! 4. **Non-terminal**: Raw mode is never enabled when stdout is not a TTY
//!
//! This pattern is consistent with [`crate::replay_countdown`], which uses
//! the same approach for keyboard input during replay delays.
//!
//! # Keyboard Polling
//!
//! Keyboard polling uses `event::poll(Duration::ZERO)` for non-blocking input
//! detection. This is called during the 100ms timeout between subprocess output
//! line checks, allowing responsive key handling without blocking I/O.

use super::timeout::try_wait_child;
use super::types::{
    StreamingSubprocessResult, SubprocessError, DEFAULT_GAP_THRESHOLD_MS, EXIT_CODE_HARD_STOP,
    EXIT_CODE_INTERRUPTED, EXIT_CODE_KILLED,
};
use crate::highlight::ThemeConfig;
use crate::keyboard::{check_for_run_action, RawModeGuard, RunKeyAction};
use crate::signal;
use crate::spinner::{KeyHintState, Spinner, SpinnerContext, SpinnerSessionInfo};
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

/// Result of subprocess invocation including any detected keyboard action.
///
/// This struct bundles the subprocess result with any keyboard action that
/// was detected during execution, enabling the caller to react to user input.
#[derive(Debug)]
pub struct SpinnerSubprocessOutcome {
    /// The subprocess execution result (success or error).
    pub subprocess_result: Result<StreamingSubprocessResult, SubprocessError>,
    /// Keyboard action detected during execution (if any).
    ///
    /// This is set when the user presses a control key (s, S, p) during
    /// subprocess execution. The caller should check this field to handle
    /// soft stop, hard stop, or pause actions.
    pub key_action: Option<RunKeyAction>,
}

/// Manages raw mode state for keyboard input during subprocess execution.
///
/// This is kept separate from `SpinnerSubprocessConfig` because it tracks
/// mutable runtime state (raw mode guard, detected actions) rather than
/// configuration. This separation follows the Functional Core / Imperative
/// Shell pattern.
///
/// Raw mode is enabled only during spinner display periods to allow
/// non-blocking keyboard polling. It is disabled before any subprocess
/// output is written to avoid corrupting terminal output.
struct KeyboardState {
    /// Whether we're in an interactive terminal.
    is_terminal: bool,
    /// Current raw mode guard (Some when raw mode is active).
    raw_mode_guard: Option<RawModeGuard>,
    /// Last detected keyboard action (for returning to caller).
    detected_action: Option<RunKeyAction>,
    /// Whether output is currently paused.
    is_paused: bool,
    /// Buffer for output while paused.
    pause_buffer: Vec<String>,
}

impl KeyboardState {
    /// Create new keyboard state.
    fn new(is_terminal: bool) -> Self {
        Self {
            is_terminal,
            raw_mode_guard: None,
            detected_action: None,
            is_paused: false,
            pause_buffer: Vec::new(),
        }
    }

    /// Enable raw mode for keyboard input (during spinner display).
    ///
    /// This is a no-op if already enabled or if not in a terminal.
    fn enable_raw_mode(&mut self) {
        if !self.is_terminal || self.raw_mode_guard.is_some() {
            return;
        }
        self.raw_mode_guard = Some(RawModeGuard::new());
    }

    /// Disable raw mode before subprocess output.
    ///
    /// Raw mode must be disabled before writing subprocess output to
    /// prevent terminal corruption.
    fn disable_raw_mode(&mut self) {
        // Dropping the guard disables raw mode
        self.raw_mode_guard = None;
    }

    /// Poll for keyboard input (non-blocking).
    ///
    /// Returns true if a significant action was detected that the caller
    /// should handle. Raw mode must be enabled for this to work.
    fn poll(&mut self) -> bool {
        if self.raw_mode_guard.is_none() {
            return false;
        }

        let action = check_for_run_action();
        if action != RunKeyAction::None {
            self.detected_action = Some(action);
            true
        } else {
            false
        }
    }

    /// Get and clear the detected action (if any).
    ///
    /// Returns the keyboard action that was detected during polling,
    /// clearing it so subsequent calls return None.
    fn take_action(&mut self) -> Option<RunKeyAction> {
        self.detected_action.take()
    }

    /// Check if the detected action matches a specific action.
    fn matches_action(&self, action: RunKeyAction) -> bool {
        self.detected_action == Some(action)
    }

    /// Clear the detected action without returning it.
    fn clear_action(&mut self) {
        self.detected_action = None;
    }

    /// Toggle the pause state.
    ///
    /// When pausing, returns an empty Vec.
    /// When resuming, returns buffered output for immediate display.
    fn toggle_pause(&mut self) -> Vec<String> {
        self.is_paused = !self.is_paused;
        if self.is_paused {
            Vec::new()
        } else {
            std::mem::take(&mut self.pause_buffer)
        }
    }

    /// Check if currently paused.
    fn is_paused(&self) -> bool {
        self.is_paused
    }

    /// Buffer output for later display when paused.
    fn buffer_output(&mut self, output: String) {
        self.pause_buffer.push(output);
    }

    /// Drain any remaining buffered output (called on exit).
    fn drain_buffer(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pause_buffer)
    }
}

/// Drains the pause buffer, printing any buffered output.
///
/// Called during exit handling to ensure output buffered while paused
/// is displayed before the subprocess result is returned.
fn drain_pause_buffer(keyboard: &mut KeyboardState) {
    let buffered = keyboard.drain_buffer();
    if !buffered.is_empty() {
        for output in buffered {
            print!("{}", output);
        }
        let _ = io::stdout().flush();
    }
}

/// Drains remaining stdout lines from the channel and processes them.
///
/// Called during exit handling to ensure all buffered output is displayed
/// before the subprocess result is returned.
fn drain_stdout(line_rx: &mpsc::Receiver<io::Result<String>>, processor: &mut StreamProcessor) {
    while let Ok(line_result) = line_rx.try_recv() {
        if let Ok(line) = line_result {
            if let Some(output) = processor.process_line(&line) {
                print!("{}", output);
                let _ = io::stdout().flush();
            }
        }
    }
}

/// Drains remaining stderr lines from the channel.
fn drain_stderr(stderr_rx: &mpsc::Receiver<String>) {
    while let Ok(line) = stderr_rx.try_recv() {
        eprintln!("{}", line);
    }
}

/// Waits for output threads to finish and returns the captured stderr.
fn join_output_threads(
    stdout_thread: thread::JoinHandle<()>,
    stderr_thread: thread::JoinHandle<String>,
) -> String {
    let _ = stdout_thread.join();
    stderr_thread.join().unwrap_or_default()
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
/// # Terminal Raw Mode
///
/// Raw mode is automatically managed during execution to enable keyboard input
/// polling without corrupting subprocess output:
/// - Enabled during spinner display to allow non-blocking keyboard polling
/// - Disabled before any subprocess output is written to prevent corruption
/// - Disabled on all exit paths (completion, timeout, interrupt, error)
/// - Skipped entirely when stdout is not a terminal (non-interactive mode)
///
/// # Keyboard Polling
///
/// During spinner display periods, the function polls for keyboard input using
/// non-blocking `event::poll(Duration::ZERO)`. Any detected key action (soft stop,
/// hard stop, pause) is captured and returned in the outcome, allowing callers
/// to react to user input.
///
/// # Arguments
///
/// * `config` - Configuration for the subprocess invocation
///
/// # Returns
///
/// Returns a [`SpinnerSubprocessOutcome`] containing both the subprocess result
/// and any keyboard action detected during execution.
///
/// # Example
///
/// ```no_run
/// use ralph::subprocess::{invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig};
/// use ralph::highlight::ThemeConfig;
/// use ralph::stream_processor::VerboseToolsConfig;
/// use ralph::spinner::SpinnerSessionInfo;
/// use ralph::keyboard::RunKeyAction;
///
/// let config = SpinnerSubprocessConfig {
///     command: "claude --output-format stream-json -p 'hello'".to_string(),
///     timeout_secs: 300,
///     theme_config: ThemeConfig::new().with_theme("Monokai Extended"),
///     session_elapsed_ms: 0,
///     verbose_tools: VerboseToolsConfig::new(),
///     session_info: SpinnerSessionInfo::new("brave-panda".to_string(), 1, 5),
/// };
/// let outcome = invoke_subprocess_with_spinner_config(&config);
///
/// // Check for keyboard action
/// if let Some(action) = outcome.key_action {
///     match action {
///         RunKeyAction::SoftStop => println!("User requested soft stop"),
///         RunKeyAction::HardStop => println!("User requested hard stop"),
///         RunKeyAction::Pause => println!("User requested pause"),
///         RunKeyAction::None => {}
///     }
/// }
/// ```
pub fn invoke_subprocess_with_spinner_config(
    config: &SpinnerSubprocessConfig,
) -> SpinnerSubprocessOutcome {
    let mut keyboard = KeyboardState::new(std::io::stdout().is_terminal());
    let subprocess_result = run_subprocess_with_spinner(config, &mut keyboard);
    SpinnerSubprocessOutcome {
        subprocess_result,
        key_action: keyboard.take_action(),
    }
}

/// Internal helper that runs the subprocess and tracks keyboard state.
fn run_subprocess_with_spinner(
    config: &SpinnerSubprocessConfig,
    keyboard: &mut KeyboardState,
) -> Result<StreamingSubprocessResult, SubprocessError> {
    // Spawn subprocess with stdout/stderr captured and stdin inherited
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&config.command)
        .stdin(Stdio::null()) // Null stdin so parent can capture keypresses for controls
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

    // Create stream processor with theme configuration and verbose tools
    let mut processor = StreamProcessor::with_verbose_tools(
        config.theme_config.clone(),
        keyboard.is_terminal,
        keyboard.is_terminal,
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

    // Enable raw mode to allow keyboard polling during spinner display
    keyboard.enable_raw_mode();

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
            // Disable raw mode before draining output to prevent corruption
            keyboard.disable_raw_mode();

            // Drain any output buffered while paused
            drain_pause_buffer(keyboard);

            // Drain remaining output and wait for threads
            drain_stdout(&line_rx, &mut processor);
            drain_stderr(&stderr_rx);
            let stderr_captured = join_output_threads(stdout_thread, stderr_thread);

            // Extract exit code
            let exit_code = status.code().ok_or(SubprocessError::Signaled)?;

            return Ok(StreamingSubprocessResult {
                exit_code,
                stderr: stderr_captured,
                stream_result: processor.finish(),
            });
        }

        // Check timeout
        if start.elapsed() >= timeout {
            // Stop spinner if active
            if spinner_active {
                spinner.stop();
            }
            // Disable raw mode before draining output to prevent corruption
            keyboard.disable_raw_mode();

            // Kill the subprocess
            let _ = child.kill();
            let _ = child.wait(); // Clean up zombie

            // Drain any output buffered while paused
            drain_pause_buffer(keyboard);

            // Drain remaining output and wait for threads
            drain_stdout(&line_rx, &mut processor);
            let stderr_captured = join_output_threads(stdout_thread, stderr_thread);

            return Err(SubprocessError::Timeout {
                timeout_secs: config.timeout_secs,
                partial_result: Box::new(StreamingSubprocessResult {
                    exit_code: EXIT_CODE_KILLED,
                    stderr: stderr_captured,
                    stream_result: processor.finish(),
                }),
            });
        }

        // Check for interrupt signal (SIGINT/SIGTERM)
        if signal::is_interrupted() {
            // Stop spinner if active
            if spinner_active {
                spinner.stop();
            }
            // Disable raw mode before draining output to prevent corruption
            keyboard.disable_raw_mode();

            // Kill the subprocess gracefully
            let _ = child.kill();
            let _ = child.wait(); // Clean up zombie

            // Drain any output buffered while paused
            drain_pause_buffer(keyboard);

            // Drain remaining output and wait for threads
            drain_stdout(&line_rx, &mut processor);
            let stderr_captured = join_output_threads(stdout_thread, stderr_thread);

            return Err(SubprocessError::Interrupted {
                partial_result: Box::new(StreamingSubprocessResult {
                    exit_code: EXIT_CODE_INTERRUPTED,
                    stderr: stderr_captured,
                    stream_result: processor.finish(),
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
                        // Check pause state first to avoid unnecessary clone
                        if keyboard.is_paused() {
                            // Buffer output while paused - don't print or stop spinner
                            keyboard.buffer_output(output);
                            // Still update last_output_time so we know output is arriving
                            last_output_time = Instant::now();
                        } else {
                            // Stop spinner on visible output
                            if spinner_active {
                                spinner.stop();
                                spinner_active = false;
                            }
                            // Disable raw mode before writing output to prevent corruption
                            keyboard.disable_raw_mode();
                            print!("{}", output);
                            let _ = io::stdout().flush();
                            // Update last output time
                            last_output_time = Instant::now();
                        }
                    }
                }
                Err(e) => {
                    // Stop spinner before returning error
                    if spinner_active {
                        spinner.stop();
                    }
                    // Disable raw mode before returning error
                    keyboard.disable_raw_mode();
                    // Drain any output buffered while paused
                    drain_pause_buffer(keyboard);
                    return Err(SubprocessError::OutputCaptureFailed(format!(
                        "Failed to read stdout: {}",
                        e
                    )));
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Check for stderr output (disable raw mode temporarily for clean output)
                if let Ok(line) = stderr_rx.try_recv() {
                    keyboard.disable_raw_mode();
                    eprintln!("{}", line);
                    while let Ok(line) = stderr_rx.try_recv() {
                        eprintln!("{}", line);
                    }
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
                    // Re-enable raw mode for keyboard input during spinner
                    keyboard.enable_raw_mode();
                }

                // Poll for keyboard input during wait periods
                // This allows interactive controls (soft stop, hard stop, pause)
                // to be detected without blocking subprocess I/O
                if keyboard.poll() {
                    // Check if hard stop was requested - kill subprocess immediately
                    if keyboard.matches_action(RunKeyAction::HardStop) {
                        if spinner_active {
                            spinner.stop();
                        }
                        keyboard.disable_raw_mode();

                        // Kill the subprocess
                        let _ = child.kill();
                        let _ = child.wait();

                        // Drain any output buffered while paused
                        drain_pause_buffer(keyboard);

                        // Drain remaining output and wait for threads
                        drain_stdout(&line_rx, &mut processor);
                        let stderr_captured = join_output_threads(stdout_thread, stderr_thread);

                        return Err(SubprocessError::HardStop {
                            partial_result: Box::new(StreamingSubprocessResult {
                                exit_code: EXIT_CODE_HARD_STOP,
                                stderr: stderr_captured,
                                stream_result: processor.finish(),
                            }),
                        });
                    }

                    // Check if soft stop was requested - update hints but let subprocess continue
                    // The action is NOT cleared (unlike Pause) so it propagates to caller
                    // for iteration boundary checking
                    if keyboard.matches_action(RunKeyAction::SoftStop) {
                        spinner.set_key_hint_state(KeyHintState::Finishing);
                    }

                    // Check if pause was requested - toggle pause state
                    if keyboard.matches_action(RunKeyAction::Pause) {
                        // Clear the detected action so it's not returned to caller
                        // (pause is handled locally, not propagated up)
                        keyboard.clear_action();

                        // Toggle pause and get any buffered output to display
                        let buffered = keyboard.toggle_pause();

                        // Update spinner hint to show pause state
                        if keyboard.is_paused() {
                            spinner.set_key_hint_state(KeyHintState::Paused);
                        } else {
                            spinner.set_key_hint_state(KeyHintState::Running);
                            // Flush buffered output on resume
                            if !buffered.is_empty() {
                                // Stop spinner before displaying output
                                if spinner_active {
                                    spinner.stop();
                                    spinner_active = false;
                                }
                                keyboard.disable_raw_mode();
                                for output in buffered {
                                    print!("{}", output);
                                }
                                let _ = io::stdout().flush();
                                last_output_time = Instant::now();
                            }
                        }
                    }
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
