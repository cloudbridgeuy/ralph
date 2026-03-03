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
//! - Key binding hints showing available controls (when in interactive terminal)
//!
//! # Key Binding Hints
//!
//! When running in an interactive terminal, the spinner displays key hints
//! that update based on the current state:
//!
//! - **Running**: `[s: stop | S: halt | p: pause]`
//! - **Finishing** (soft stop requested): `[finishing...]`
//! - **Paused**: `[paused - p: resume]`
//!
//! # Example
//!
//! ```no_run
//! use ralph::spinner::Spinner;
//!
//! let spinner = Spinner::new();
//! spinner.start(); // Shows: ⠋ Waiting for response... 0s [s: stop | S: halt | p: pause]
//!
//! // ... wait for output ...
//!
//! spinner.stop(); // Clears the spinner line
//! ```

use crate::ansi::{CLEAR_EOL, CR, CYAN, DIM, RESET, YELLOW};
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

/// State for key binding hints displayed in the spinner.
///
/// The hints update dynamically based on user actions:
/// - Running: Shows all available key bindings
/// - Finishing: Shows that soft stop was requested and current iteration will complete
/// - Paused: Shows pause indicator and resume key
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyHintState {
    /// Normal running state - show all available key bindings.
    #[default]
    Running,
    /// Soft stop requested - finishing current iteration.
    Finishing,
    /// Execution is paused.
    Paused,
}

impl KeyHintState {
    /// Get the key hints display string for this state.
    ///
    /// Returns the formatted hint text including ANSI colors.
    pub fn hint_text(&self) -> &'static str {
        match self {
            Self::Running => "[s: stop | S: halt | p: pause]",
            Self::Finishing => "[finishing...]",
            Self::Paused => "[paused - p: resume]",
        }
    }
}

/// Session metadata for spinner display.
///
/// Groups session context information to display alongside the spinner,
/// keeping the function signatures under 5 arguments.
#[derive(Debug, Clone, Default)]
pub struct SpinnerSessionInfo {
    /// Persona name (e.g., "developer").
    pub persona: Option<String>,
    /// Session slug/name (e.g., "brave-panda").
    pub slug: Option<String>,
    /// Current iteration number (1-indexed).
    pub current_iteration: Option<usize>,
    /// Total number of iterations for the session.
    pub max_iterations: Option<usize>,
    /// Originator persona name when invoked via orchestration.
    /// When set, the spinner shows "architect (for pm)" instead of just "architect".
    pub on_behalf_of: Option<String>,
}

impl SpinnerSessionInfo {
    /// Create new session info with all fields (without persona).
    pub fn new(slug: String, current_iteration: usize, max_iterations: usize) -> Self {
        Self {
            persona: None,
            slug: Some(slug),
            current_iteration: Some(current_iteration),
            max_iterations: Some(max_iterations),
            on_behalf_of: None,
        }
    }

    /// Check if any session info is available.
    pub fn has_info(&self) -> bool {
        self.persona.is_some() || self.slug.is_some() || self.current_iteration.is_some()
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
    /// Current state for key hint display.
    key_hint_state: Arc<Mutex<KeyHintState>>,
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
            key_hint_state: Arc::new(Mutex::new(KeyHintState::default())),
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
            key_hint_state: Arc::clone(&self.key_hint_state),
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

    /// Set the key hint state.
    ///
    /// This changes the key binding hints displayed by the spinner.
    /// Used to indicate soft stop requested (Finishing) or paused state.
    ///
    /// # Arguments
    ///
    /// * `state` - The new key hint state to display
    pub fn set_key_hint_state(&self, state: KeyHintState) {
        if let Ok(mut hint) = self.key_hint_state.lock() {
            *hint = state;
        }
    }

    /// Get the current key hint state.
    pub fn get_key_hint_state(&self) -> KeyHintState {
        self.key_hint_state.lock().map(|s| *s).unwrap_or_default()
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
    /// Current key hint state for display.
    key_hint_state: Arc<Mutex<KeyHintState>>,
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
        key_hint_state,
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

        // Get key hints based on current state
        let key_hints = key_hint_state
            .lock()
            .map(|state| format_key_hints(*state))
            .unwrap_or_default();

        // Build the spinner line
        // Use CR to return to start, then CLEAR_EOL to clear to end of line
        // This prevents residual characters when text length changes
        // (e.g., "59s" → "1m 0s" or context message changes)
        // Format: "⠋ Thinking... Session: brave-panda | Iteration: 2/5 | 12s [s: stop | S: halt | p: pause]"
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

/// Format key hints with ANSI styling based on state.
///
/// Returns the key hints string with appropriate styling:
/// - Running: dimmed hints showing all controls
/// - Finishing/Paused: yellow indicator for active states
fn format_key_hints(state: KeyHintState) -> String {
    let color = match state {
        KeyHintState::Running => DIM,
        KeyHintState::Finishing | KeyHintState::Paused => YELLOW,
    };
    format!("{color}{}{RESET}", state.hint_text())
}

/// Format session info for spinner display.
///
/// # Format
///
/// - With persona: `developer (brave-panda 2/5)` — compact format in parentheses
/// - Without persona: `Session: brave-panda | Iteration: 2/5` — verbose labeled format
/// - Returns empty string if no info is available.
fn format_session_info(info: &SpinnerSessionInfo) -> String {
    match info.persona {
        Some(ref persona) => format_session_info_with_persona(persona, info),
        None => format_session_info_without_persona(info),
    }
}

/// Format session info with a persona prefix.
///
/// Uses compact format: `developer (brave-panda 2/5)`.
/// When `on_behalf_of` is set: `architect (for pm) (brave-panda 1/?)`.
/// Session details are placed in parentheses after the persona name.
fn format_session_info_with_persona(persona: &str, info: &SpinnerSessionInfo) -> String {
    let mut detail_parts = Vec::new();

    if let Some(ref slug) = info.slug {
        detail_parts.push(slug.clone());
    }

    match (info.current_iteration, info.max_iterations) {
        (Some(current), Some(max)) => detail_parts.push(format!("{}/{}", current, max)),
        (Some(current), None) => detail_parts.push(format!("{}/?", current)),
        _ => {}
    }

    let behalf = if let Some(ref name) = info.on_behalf_of {
        format!(" (for {name})")
    } else {
        String::new()
    };

    if detail_parts.is_empty() {
        format!("{persona}{behalf}")
    } else {
        format!("{persona}{behalf} ({})", detail_parts.join(" "))
    }
}

/// Format session info without a persona.
///
/// Uses verbose labeled format: `Session: brave-panda | Iteration: 2/5`.
fn format_session_info_without_persona(info: &SpinnerSessionInfo) -> String {
    let mut parts = Vec::new();

    if let Some(ref slug) = info.slug {
        parts.push(format!("Session: {}", slug));
    }

    if let (Some(current), Some(max)) = (info.current_iteration, info.max_iterations) {
        parts.push(format!("Iteration: {}/{}", current, max));
    }

    parts.join(" | ")
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
mod tests;
