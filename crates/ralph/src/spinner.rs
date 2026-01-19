//! Spinner display for waiting states during subprocess execution.
//!
//! This module provides an animated spinner with elapsed time display while
//! waiting for LLM responses. The spinner runs on a separate thread and
//! clears itself when output begins.
//!
//! # Features
//!
//! - Animated spinner characters (braille pattern)
//! - Elapsed time display that updates every second
//! - Iteration-level and session-level time tracking
//! - Terminal detection (no spinner when piped)
//! - Thread-safe start/stop control
//!
//! # Example
//!
//! ```no_run
//! use ralph::spinner::Spinner;
//!
//! let spinner = Spinner::new();
//! spinner.start(); // Shows: ⠋ Waiting for response... 0s
//!
//! // ... wait for output ...
//!
//! spinner.stop(); // Clears the spinner line
//! ```

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Braille spinner characters for animation.
/// These create a smooth spinning effect: ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏
const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Interval between spinner frame updates.
const SPINNER_INTERVAL: Duration = Duration::from_millis(80);

/// The context or reason for showing the spinner.
///
/// Different wait states should show different messages to give users
/// better feedback about what the system is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpinnerContext {
    /// Initial wait for LLM to start responding.
    #[default]
    WaitingForResponse,
    /// LLM is thinking between outputs (gap in streaming).
    Thinking,
    /// Waiting for a tool to complete execution.
    WaitingForTool,
    /// Buffering output (e.g., waiting for code block to close).
    Buffering,
    /// Summarizing the progress file.
    Summarizing,
}

impl SpinnerContext {
    /// Get the display message for this context.
    pub fn message(&self) -> &'static str {
        match self {
            Self::WaitingForResponse => "Waiting for response...",
            Self::Thinking => "Thinking...",
            Self::WaitingForTool => "Running tool...",
            Self::Buffering => "Buffering code...",
            Self::Summarizing => "Summarizing progress file...",
        }
    }
}

/// A spinner that displays while waiting for LLM responses.
///
/// The spinner runs on a background thread and shows:
/// - An animated spinner character
/// - A contextual message (e.g., "Waiting for response...", "Thinking...")
/// - Elapsed time in seconds or minutes:seconds format
///
/// Call [`start`](Spinner::start) to begin spinning and [`stop`](Spinner::stop)
/// to clear the spinner when output arrives.
pub struct Spinner {
    /// Flag to signal the spinner thread to stop.
    running: Arc<AtomicBool>,
    /// Handle to the spinner thread (if running).
    thread_handle: Option<JoinHandle<()>>,
    /// Whether spinner is enabled (terminal detection).
    enabled: bool,
    /// Start time of the current iteration.
    iteration_start: Instant,
    /// Total elapsed time from previous iterations in this session.
    session_elapsed_ms: u64,
    /// Current context for the spinner message.
    context: Arc<std::sync::Mutex<SpinnerContext>>,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    /// Create a new spinner.
    ///
    /// The spinner is enabled only when stdout is a terminal.
    /// When piped, all spinner methods are no-ops.
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
            enabled: std::io::stdout().is_terminal(),
            iteration_start: Instant::now(),
            session_elapsed_ms: 0,
            context: Arc::new(std::sync::Mutex::new(SpinnerContext::default())),
        }
    }

    /// Create a spinner with custom enable state.
    ///
    /// Useful for testing or forcing spinner behavior.
    pub fn with_enabled(enabled: bool) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
            enabled,
            iteration_start: Instant::now(),
            session_elapsed_ms: 0,
            context: Arc::new(std::sync::Mutex::new(SpinnerContext::default())),
        }
    }

    /// Create a spinner with session elapsed time from previous iterations.
    ///
    /// # Arguments
    ///
    /// * `session_elapsed_ms` - Accumulated time from previous iterations
    pub fn with_session_elapsed(session_elapsed_ms: u64) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
            enabled: std::io::stdout().is_terminal(),
            iteration_start: Instant::now(),
            session_elapsed_ms,
            context: Arc::new(std::sync::Mutex::new(SpinnerContext::default())),
        }
    }

    /// Check if the spinner is currently running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Check if the spinner is enabled (terminal mode).
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Start the spinner animation.
    ///
    /// If the spinner is disabled (non-terminal) or already running, this is a no-op.
    /// The spinner will continue until [`stop`](Spinner::stop) is called.
    pub fn start(&mut self) {
        self.start_with_context(SpinnerContext::default());
    }

    /// Start the spinner animation with a specific context message.
    ///
    /// If the spinner is disabled (non-terminal) or already running, this is a no-op.
    /// The spinner will continue until [`stop`](Spinner::stop) is called.
    ///
    /// Note: This does NOT reset iteration start time. Use this for restarting
    /// the spinner after gaps during an iteration. The iteration time continues
    /// accumulating from when the iteration started.
    ///
    /// # Arguments
    ///
    /// * `context` - The context determining what message to display
    pub fn start_with_context(&mut self, context: SpinnerContext) {
        if !self.enabled || self.is_running() {
            return;
        }

        // Note: We do NOT reset iteration_start here.
        // The iteration time should continue from when the iteration actually started,
        // not from when the spinner was restarted after a gap.

        // Set context
        if let Ok(mut ctx) = self.context.lock() {
            *ctx = context;
        }

        // Set running flag
        self.running.store(true, Ordering::SeqCst);

        // Clone shared state for the thread
        let running = Arc::clone(&self.running);
        let iteration_start = self.iteration_start;
        let session_elapsed_ms = self.session_elapsed_ms;
        let context_arc = Arc::clone(&self.context);

        // Spawn spinner thread
        let handle = thread::spawn(move || {
            run_spinner(running, iteration_start, session_elapsed_ms, context_arc);
        });

        self.thread_handle = Some(handle);
    }

    /// Update the spinner context while it's running.
    ///
    /// This changes the message displayed by the spinner without stopping it.
    /// If the spinner is not running, this has no effect.
    ///
    /// # Arguments
    ///
    /// * `context` - The new context determining what message to display
    pub fn set_context(&self, context: SpinnerContext) {
        if let Ok(mut ctx) = self.context.lock() {
            *ctx = context;
        }
    }

    /// Get the current spinner context.
    pub fn get_context(&self) -> SpinnerContext {
        self.context.lock().map(|ctx| *ctx).unwrap_or_default()
    }

    /// Stop the spinner and clear the display.
    ///
    /// If the spinner is not running, this is a no-op.
    /// This method blocks until the spinner thread has exited.
    pub fn stop(&mut self) {
        if !self.is_running() {
            return;
        }

        // Signal the thread to stop
        self.running.store(false, Ordering::SeqCst);

        // Wait for the thread to finish
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }

        // Clear the spinner line (only if enabled)
        if self.enabled {
            clear_spinner_line();
        }
    }

    /// Get the elapsed time for the current iteration in milliseconds.
    pub fn iteration_elapsed_ms(&self) -> u64 {
        self.iteration_start.elapsed().as_millis() as u64
    }

    /// Get the total session elapsed time in milliseconds.
    ///
    /// This includes time from previous iterations plus the current iteration.
    pub fn total_session_elapsed_ms(&self) -> u64 {
        self.session_elapsed_ms + self.iteration_elapsed_ms()
    }

    /// Update the session elapsed time (call between iterations).
    ///
    /// This should be called after each iteration completes to accumulate
    /// the iteration time into the session total.
    pub fn accumulate_iteration_time(&mut self) {
        self.session_elapsed_ms += self.iteration_elapsed_ms();
        self.iteration_start = Instant::now();
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Run the spinner animation loop.
///
/// This function runs on a background thread and displays the spinner
/// until the running flag is set to false.
fn run_spinner(
    running: Arc<AtomicBool>,
    iteration_start: Instant,
    session_elapsed_ms: u64,
    context: Arc<std::sync::Mutex<SpinnerContext>>,
) {
    let mut frame = 0;
    let mut stdout = std::io::stdout();

    while running.load(Ordering::SeqCst) {
        // Calculate elapsed times
        let iteration_elapsed = iteration_start.elapsed();
        let iteration_secs = iteration_elapsed.as_secs();
        let total_elapsed_ms = session_elapsed_ms + iteration_elapsed.as_millis() as u64;

        // Format the time display
        let time_display = format_spinner_time(iteration_secs, total_elapsed_ms);

        // Get current spinner character
        let spinner_char = SPINNER_CHARS[frame % SPINNER_CHARS.len()];

        // Get the context message
        let message = context
            .lock()
            .map(|ctx| ctx.message())
            .unwrap_or("Working...");

        // Build the spinner line
        let spinner_line = format!(
            "\r\x1b[36m{}\x1b[0m {} {}",
            spinner_char, message, time_display
        );

        // Write and flush
        let _ = write!(stdout, "{}", spinner_line);
        let _ = stdout.flush();

        // Advance frame
        frame += 1;

        // Sleep for the interval
        thread::sleep(SPINNER_INTERVAL);
    }
}

/// Format elapsed time for spinner display.
///
/// Shows both iteration time and total session time when they differ.
///
/// # Format
///
/// - Just iteration time: "12s" or "1m 23s"
/// - With session time: "12s • Total: 1m 45s"
fn format_spinner_time(iteration_secs: u64, total_elapsed_ms: u64) -> String {
    let iteration_display = format_time_short(iteration_secs);
    let total_secs = total_elapsed_ms / 1000;

    // Only show session total if it differs significantly from iteration time
    // (more than 1 second difference, indicating multiple iterations)
    if total_secs > iteration_secs + 1 {
        let total_display = format_time_short(total_secs);
        format!("{} • Total: {}", iteration_display, total_display)
    } else {
        iteration_display
    }
}

/// Format seconds into short time display.
///
/// # Format
///
/// - 0-59s: "12s"
/// - 60s+: "1m 23s"
fn format_time_short(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else {
        let mins = secs / 60;
        let remaining_secs = secs % 60;
        format!("{}m {}s", mins, remaining_secs)
    }
}

/// Clear the spinner line from the terminal.
fn clear_spinner_line() {
    let mut stdout = std::io::stdout();
    // Move to beginning of line, clear to end of line
    let _ = write!(stdout, "\r\x1b[K");
    let _ = stdout.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_default() {
        let spinner = Spinner::default();
        // Should not be running initially
        assert!(!spinner.is_running());
    }

    #[test]
    fn test_spinner_new() {
        let spinner = Spinner::new();
        assert!(!spinner.is_running());
    }

    #[test]
    fn test_spinner_with_enabled_false() {
        let spinner = Spinner::with_enabled(false);
        assert!(!spinner.is_enabled());
        assert!(!spinner.is_running());
    }

    #[test]
    fn test_spinner_with_enabled_true() {
        let spinner = Spinner::with_enabled(true);
        assert!(spinner.is_enabled());
        assert!(!spinner.is_running());
    }

    #[test]
    fn test_spinner_with_session_elapsed() {
        let spinner = Spinner::with_session_elapsed(5000);
        assert_eq!(spinner.session_elapsed_ms, 5000);
    }

    #[test]
    fn test_spinner_disabled_start_stop() {
        // Disabled spinner should be a no-op
        let mut spinner = Spinner::with_enabled(false);
        spinner.start();
        assert!(!spinner.is_running()); // Should not actually start
        spinner.stop(); // Should be safe to call
    }

    #[test]
    fn test_spinner_enabled_start_stop() {
        let mut spinner = Spinner::with_enabled(true);
        spinner.start();
        // Give thread a moment to start
        thread::sleep(Duration::from_millis(10));
        assert!(spinner.is_running());
        spinner.stop();
        assert!(!spinner.is_running());
    }

    #[test]
    fn test_spinner_double_start() {
        let mut spinner = Spinner::with_enabled(true);
        spinner.start();
        spinner.start(); // Should be no-op
        thread::sleep(Duration::from_millis(10));
        assert!(spinner.is_running());
        spinner.stop();
    }

    #[test]
    fn test_spinner_double_stop() {
        let mut spinner = Spinner::with_enabled(true);
        spinner.start();
        thread::sleep(Duration::from_millis(10));
        spinner.stop();
        spinner.stop(); // Should be safe to call multiple times
        assert!(!spinner.is_running());
    }

    #[test]
    fn test_spinner_drop_stops() {
        let running = {
            let mut spinner = Spinner::with_enabled(true);
            spinner.start();
            thread::sleep(Duration::from_millis(10));
            Arc::clone(&spinner.running)
            // spinner dropped here
        };
        // Thread should have been stopped
        thread::sleep(Duration::from_millis(50));
        assert!(!running.load(Ordering::SeqCst));
    }

    #[test]
    fn test_format_time_short_seconds() {
        assert_eq!(format_time_short(0), "0s");
        assert_eq!(format_time_short(1), "1s");
        assert_eq!(format_time_short(30), "30s");
        assert_eq!(format_time_short(59), "59s");
    }

    #[test]
    fn test_format_time_short_minutes() {
        assert_eq!(format_time_short(60), "1m 0s");
        assert_eq!(format_time_short(90), "1m 30s");
        assert_eq!(format_time_short(125), "2m 5s");
        assert_eq!(format_time_short(3600), "60m 0s");
    }

    #[test]
    fn test_format_spinner_time_iteration_only() {
        // When session time equals iteration time, show only iteration
        let result = format_spinner_time(12, 12_000);
        assert_eq!(result, "12s");
    }

    #[test]
    fn test_format_spinner_time_with_session() {
        // When session time differs significantly, show both
        let result = format_spinner_time(12, 105_000);
        assert_eq!(result, "12s • Total: 1m 45s");
    }

    #[test]
    fn test_format_spinner_time_small_difference() {
        // Small difference (1 second) should not show session time
        let result = format_spinner_time(12, 13_000);
        assert_eq!(result, "12s");
    }

    #[test]
    fn test_spinner_iteration_elapsed() {
        let spinner = Spinner::with_enabled(false);
        thread::sleep(Duration::from_millis(50));
        let elapsed = spinner.iteration_elapsed_ms();
        assert!(elapsed >= 50);
        assert!(elapsed < 150); // Reasonable upper bound
    }

    #[test]
    fn test_spinner_total_session_elapsed() {
        let mut spinner = Spinner::with_enabled(false);
        spinner.session_elapsed_ms = 10_000;
        thread::sleep(Duration::from_millis(50));
        let total = spinner.total_session_elapsed_ms();
        assert!(total >= 10_050);
        assert!(total < 10_150);
    }

    #[test]
    fn test_spinner_accumulate_iteration_time() {
        let mut spinner = Spinner::with_enabled(false);
        thread::sleep(Duration::from_millis(100));
        spinner.accumulate_iteration_time();
        // Session elapsed should now have the iteration time
        assert!(spinner.session_elapsed_ms >= 100);
        assert!(spinner.session_elapsed_ms < 200);
        // New iteration should start fresh
        let elapsed = spinner.iteration_elapsed_ms();
        assert!(elapsed < 50); // Should be near zero
    }

    #[test]
    fn test_spinner_chars_count() {
        // Verify we have enough spinner chars for smooth animation
        assert!(SPINNER_CHARS.len() >= 8); // Should have good variety (currently 10)
        assert_eq!(SPINNER_CHARS.len(), 10); // Document expected count
    }

    // Context-related tests

    #[test]
    fn test_spinner_context_default() {
        assert_eq!(
            SpinnerContext::default(),
            SpinnerContext::WaitingForResponse
        );
    }

    #[test]
    fn test_spinner_context_messages() {
        assert_eq!(
            SpinnerContext::WaitingForResponse.message(),
            "Waiting for response..."
        );
        assert_eq!(SpinnerContext::Thinking.message(), "Thinking...");
        assert_eq!(SpinnerContext::WaitingForTool.message(), "Running tool...");
        assert_eq!(SpinnerContext::Buffering.message(), "Buffering code...");
        assert_eq!(
            SpinnerContext::Summarizing.message(),
            "Summarizing progress file..."
        );
    }

    #[test]
    fn test_spinner_get_context_default() {
        let spinner = Spinner::with_enabled(false);
        assert_eq!(spinner.get_context(), SpinnerContext::WaitingForResponse);
    }

    #[test]
    fn test_spinner_set_context() {
        let spinner = Spinner::with_enabled(false);
        assert_eq!(spinner.get_context(), SpinnerContext::WaitingForResponse);
        spinner.set_context(SpinnerContext::Thinking);
        assert_eq!(spinner.get_context(), SpinnerContext::Thinking);
        spinner.set_context(SpinnerContext::WaitingForTool);
        assert_eq!(spinner.get_context(), SpinnerContext::WaitingForTool);
    }

    #[test]
    fn test_spinner_start_with_context() {
        let mut spinner = Spinner::with_enabled(true);
        spinner.start_with_context(SpinnerContext::Thinking);
        thread::sleep(Duration::from_millis(10));
        assert!(spinner.is_running());
        assert_eq!(spinner.get_context(), SpinnerContext::Thinking);
        spinner.stop();
        assert!(!spinner.is_running());
    }

    #[test]
    fn test_spinner_context_changes_while_running() {
        let mut spinner = Spinner::with_enabled(true);
        spinner.start();
        thread::sleep(Duration::from_millis(10));
        assert!(spinner.is_running());

        // Change context while running
        spinner.set_context(SpinnerContext::WaitingForTool);
        assert_eq!(spinner.get_context(), SpinnerContext::WaitingForTool);

        // Change again
        spinner.set_context(SpinnerContext::Buffering);
        assert_eq!(spinner.get_context(), SpinnerContext::Buffering);

        spinner.stop();
    }
}
