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
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Braille spinner characters for animation.
/// These create a smooth spinning effect: ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏
pub const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Interval between spinner frame updates.
pub const SPINNER_INTERVAL: Duration = Duration::from_millis(80);

// ANSI escape sequences for terminal control
/// Carriage return - moves cursor to beginning of line.
const CR: &str = "\r";
/// Clear from cursor to end of line.
const CLEAR_EOL: &str = "\x1b[K";
/// Set foreground color to cyan.
const CYAN: &str = "\x1b[36m";
/// Reset all text formatting.
const RESET: &str = "\x1b[0m";

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

/// Session metadata for spinner display.
///
/// Groups session context information to display alongside the spinner,
/// keeping the function signatures under 5 arguments.
#[derive(Debug, Clone, Default)]
pub struct SpinnerSessionInfo {
    /// Session slug/name (e.g., "brave-panda").
    pub slug: Option<String>,
    /// Current iteration number (1-indexed).
    pub current_iteration: Option<usize>,
    /// Total number of iterations for the session.
    pub max_iterations: Option<usize>,
}

impl SpinnerSessionInfo {
    /// Create new session info with all fields.
    pub fn new(slug: String, current_iteration: usize, max_iterations: usize) -> Self {
        Self {
            slug: Some(slug),
            current_iteration: Some(current_iteration),
            max_iterations: Some(max_iterations),
        }
    }

    /// Check if any session info is available.
    pub fn has_info(&self) -> bool {
        self.slug.is_some() || self.current_iteration.is_some()
    }
}

/// A spinner that displays while waiting for LLM responses.
///
/// The spinner runs on a background thread and shows:
/// - An animated spinner character
/// - A contextual message (e.g., "Waiting for response...", "Thinking...")
/// - Session name and iteration progress (if provided)
/// - Elapsed time in seconds or minutes:seconds format
/// - Key binding hints (dimmed) showing available controls
///
/// When a soft stop is requested, the key hints change to show "[finishing...]"
/// to indicate that the current iteration will complete before pausing.
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
    context: Arc<Mutex<SpinnerContext>>,
    /// Session metadata for display.
    session_info: Arc<SpinnerSessionInfo>,
    /// Whether a soft stop has been requested.
    soft_stop_requested: Arc<AtomicBool>,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    /// Internal constructor with all configuration options.
    fn create(enabled: bool, session_elapsed_ms: u64, session_info: SpinnerSessionInfo) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
            enabled,
            iteration_start: Instant::now(),
            session_elapsed_ms,
            context: Arc::new(Mutex::new(SpinnerContext::default())),
            session_info: Arc::new(session_info),
            soft_stop_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a new spinner.
    ///
    /// The spinner is enabled only when stdout is a terminal.
    /// When piped, all spinner methods are no-ops.
    pub fn new() -> Self {
        Self::create(
            std::io::stdout().is_terminal(),
            0,
            SpinnerSessionInfo::default(),
        )
    }

    /// Create a spinner with custom enable state.
    ///
    /// Useful for testing or forcing spinner behavior.
    pub fn with_enabled(enabled: bool) -> Self {
        Self::create(enabled, 0, SpinnerSessionInfo::default())
    }

    /// Create a spinner with session elapsed time from previous iterations.
    ///
    /// # Arguments
    ///
    /// * `session_elapsed_ms` - Accumulated time from previous iterations
    pub fn with_session_elapsed(session_elapsed_ms: u64) -> Self {
        Self::create(
            std::io::stdout().is_terminal(),
            session_elapsed_ms,
            SpinnerSessionInfo::default(),
        )
    }

    /// Create a spinner with full session context.
    ///
    /// # Arguments
    ///
    /// * `session_elapsed_ms` - Accumulated time from previous iterations
    /// * `session_info` - Session metadata (slug, iteration numbers)
    pub fn with_session_context(session_elapsed_ms: u64, session_info: SpinnerSessionInfo) -> Self {
        Self::create(
            std::io::stdout().is_terminal(),
            session_elapsed_ms,
            session_info,
        )
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
        let config = SpinnerThreadConfig {
            running: Arc::clone(&self.running),
            iteration_start: self.iteration_start,
            session_elapsed_ms: self.session_elapsed_ms,
            context: Arc::clone(&self.context),
            session_info: Arc::clone(&self.session_info),
            soft_stop_requested: Arc::clone(&self.soft_stop_requested),
        };

        // Spawn spinner thread
        let handle = thread::spawn(move || {
            run_spinner(config);
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

    /// Set the soft stop requested flag.
    ///
    /// When set to true, the spinner will display "[finishing...]" instead of
    /// the key binding hints to indicate a soft stop is pending.
    pub fn set_soft_stop_requested(&self, requested: bool) {
        self.soft_stop_requested.store(requested, Ordering::SeqCst);
    }

    /// Check if a soft stop has been requested.
    pub fn is_soft_stop_requested(&self) -> bool {
        self.soft_stop_requested.load(Ordering::SeqCst)
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

/// Internal configuration for the spinner thread.
///
/// Groups parameters to keep the `run_spinner` function under 5 arguments.
/// This struct is not exposed in the public API.
struct SpinnerThreadConfig {
    /// Flag to signal the spinner thread to stop.
    running: Arc<AtomicBool>,
    /// Start time of the current iteration.
    iteration_start: Instant,
    /// Total elapsed time from previous iterations in this session.
    session_elapsed_ms: u64,
    /// Current context for the spinner message.
    context: Arc<Mutex<SpinnerContext>>,
    /// Session metadata for display.
    session_info: Arc<SpinnerSessionInfo>,
    /// Whether a soft stop has been requested.
    soft_stop_requested: Arc<AtomicBool>,
}

/// Run the spinner animation loop.
///
/// This function runs on a background thread and displays the spinner
/// until the running flag is set to false.
fn run_spinner(config: SpinnerThreadConfig) {
    let SpinnerThreadConfig {
        running,
        iteration_start,
        session_elapsed_ms,
        context,
        session_info,
        soft_stop_requested,
    } = config;
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

        // Format session info if available
        let session_display = format_session_info(&session_info);

        // Format key binding hints
        let is_soft_stop = soft_stop_requested.load(Ordering::SeqCst);
        let key_hints = format_key_hints(is_soft_stop);

        // Build the spinner line
        // Use CR to return to start, then CLEAR_EOL to clear to end of line
        // This prevents residual characters when text length changes
        // (e.g., "59s" → "1m 0s" or context message changes)
        // Format: "⠋ Thinking... Session: brave-panda | Iteration: 2/5 | 12s • Total: 1m 45s [s: pause | S: stop]"
        let spinner_line = if session_display.is_empty() {
            format!(
                "{CR}{CLEAR_EOL}{CYAN}{}{RESET} {} {} {}",
                spinner_char, message, time_display, key_hints
            )
        } else {
            format!(
                "{CR}{CLEAR_EOL}{CYAN}{}{RESET} {} {} | {} {}",
                spinner_char, message, session_display, time_display, key_hints
            )
        };

        // Write and flush
        let _ = write!(stdout, "{}", spinner_line);
        let _ = stdout.flush();

        // Advance frame
        frame += 1;

        // Sleep for the interval
        thread::sleep(SPINNER_INTERVAL);
    }
}

/// Format session info for spinner display.
///
/// # Format
///
/// Returns segments like "Session: brave-panda | Iteration: 2/5" or empty string if no info.
fn format_session_info(info: &SpinnerSessionInfo) -> String {
    let mut parts = Vec::new();

    if let Some(ref slug) = info.slug {
        parts.push(format!("Session: {}", slug));
    }

    if let (Some(current), Some(max)) = (info.current_iteration, info.max_iterations) {
        parts.push(format!("Iteration: {}/{}", current, max));
    }

    parts.join(" | ")
}

/// Key hints displayed during normal execution (dimmed).
const KEY_HINTS_NORMAL: &str = "\x1b[2m[s: pause | S: stop]\x1b[0m";
/// Key hints displayed when soft stop is requested (dimmed).
const KEY_HINTS_FINISHING: &str = "\x1b[2m[finishing...]\x1b[0m";

/// Format key binding hints for spinner display.
///
/// Shows available keyboard controls in a dimmed style so they don't distract
/// from the main status information.
///
/// # Arguments
///
/// * `soft_stop_requested` - Whether a soft stop has been requested
///
/// # Returns
///
/// - Normal state: "[s: pause | S: stop]" (dimmed)
/// - Soft stop requested: "[finishing...]" (dimmed)
fn format_key_hints(soft_stop_requested: bool) -> &'static str {
    if soft_stop_requested {
        KEY_HINTS_FINISHING
    } else {
        KEY_HINTS_NORMAL
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
    let _ = write!(stdout, "{CR}{CLEAR_EOL}");
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

    // Session info tests

    #[test]
    fn test_spinner_session_info_default() {
        let info = SpinnerSessionInfo::default();
        assert!(info.slug.is_none());
        assert!(info.current_iteration.is_none());
        assert!(info.max_iterations.is_none());
        assert!(!info.has_info());
    }

    #[test]
    fn test_spinner_session_info_new() {
        let info = SpinnerSessionInfo::new("brave-panda".to_string(), 2, 5);
        assert_eq!(info.slug, Some("brave-panda".to_string()));
        assert_eq!(info.current_iteration, Some(2));
        assert_eq!(info.max_iterations, Some(5));
        assert!(info.has_info());
    }

    #[test]
    fn test_spinner_session_info_partial() {
        let info = SpinnerSessionInfo {
            slug: Some("test-session".to_string()),
            ..Default::default()
        };
        assert!(info.has_info());

        let info2 = SpinnerSessionInfo {
            current_iteration: Some(1),
            ..Default::default()
        };
        assert!(info2.has_info());
    }

    #[test]
    fn test_format_session_info_empty() {
        let info = SpinnerSessionInfo::default();
        let display = format_session_info(&info);
        assert_eq!(display, "");
    }

    #[test]
    fn test_format_session_info_full() {
        let info = SpinnerSessionInfo::new("brave-panda".to_string(), 2, 5);
        let display = format_session_info(&info);
        assert_eq!(display, "Session: brave-panda | Iteration: 2/5");
    }

    #[test]
    fn test_format_session_info_slug_only() {
        let info = SpinnerSessionInfo {
            slug: Some("test-session".to_string()),
            ..Default::default()
        };
        let display = format_session_info(&info);
        assert_eq!(display, "Session: test-session");
    }

    #[test]
    fn test_format_session_info_iteration_only() {
        let info = SpinnerSessionInfo {
            current_iteration: Some(3),
            max_iterations: Some(10),
            ..Default::default()
        };
        let display = format_session_info(&info);
        assert_eq!(display, "Iteration: 3/10");
    }

    #[test]
    fn test_spinner_with_session_context() {
        let info = SpinnerSessionInfo::new("brave-panda".to_string(), 1, 3);
        let spinner = Spinner::with_session_context(5000, info);
        assert_eq!(spinner.session_elapsed_ms, 5000);
        assert!(spinner.session_info.has_info());
        assert_eq!(spinner.session_info.slug, Some("brave-panda".to_string()));
    }

    // Key hints tests

    #[test]
    fn test_format_key_hints_normal() {
        let hints = format_key_hints(false);
        // Should return the normal key hints constant
        assert_eq!(hints, KEY_HINTS_NORMAL);
        // Should contain the key bindings
        assert!(hints.contains("s: pause"));
        assert!(hints.contains("S: stop"));
    }

    #[test]
    fn test_format_key_hints_soft_stop_requested() {
        let hints = format_key_hints(true);
        // Should return the finishing key hints constant
        assert_eq!(hints, KEY_HINTS_FINISHING);
        // Should show finishing message instead of key bindings
        assert!(hints.contains("finishing..."));
        assert!(!hints.contains("s: pause"));
        assert!(!hints.contains("S: stop"));
    }

    // Soft stop flag tests

    #[test]
    fn test_spinner_soft_stop_default_false() {
        let spinner = Spinner::with_enabled(false);
        assert!(!spinner.is_soft_stop_requested());
    }

    #[test]
    fn test_spinner_set_soft_stop_requested() {
        let spinner = Spinner::with_enabled(false);
        assert!(!spinner.is_soft_stop_requested());

        spinner.set_soft_stop_requested(true);
        assert!(spinner.is_soft_stop_requested());

        spinner.set_soft_stop_requested(false);
        assert!(!spinner.is_soft_stop_requested());
    }

    #[test]
    fn test_spinner_soft_stop_while_running() {
        let mut spinner = Spinner::with_enabled(true);
        spinner.start();
        thread::sleep(Duration::from_millis(10));
        assert!(spinner.is_running());
        assert!(!spinner.is_soft_stop_requested());

        // Set soft stop while running
        spinner.set_soft_stop_requested(true);
        assert!(spinner.is_soft_stop_requested());

        // Spinner should still be running
        assert!(spinner.is_running());

        spinner.stop();
        assert!(!spinner.is_running());
    }
}
