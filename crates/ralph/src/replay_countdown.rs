//! Countdown display for replay delays.
//!
//! This module provides an animated countdown spinner that displays between
//! output blocks during replay when the `--delay` flag is used.
//!
//! # Features
//!
//! - Animated spinner with countdown timer
//! - Updates every second with remaining time
//! - Skip key support (press 'n' or Space to skip to next block)
//! - Pause/resume support (press 'p' to toggle)
//! - Clears automatically before the next block
//! - No display when delay is 0 or stdout is not a terminal
//!
//! # Example
//!
//! ```no_run
//! use ralph::replay_countdown::wait_with_countdown;
//!
//! // Wait 3 seconds with countdown display
//! // - Press 'n' or Space to skip to next block
//! // - Press 'p' to pause/resume
//! wait_with_countdown(3.0, true);
//! ```

use crate::spinner::{SPINNER_CHARS, SPINNER_INTERVAL};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

// ANSI escape sequences
const CR: &str = "\r";
const CLEAR_EOL: &str = "\x1b[K";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

// Pause indicator character
const PAUSE_CHAR: char = '⏸';

/// The result of waiting with countdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum CountdownResult {
    /// The countdown completed naturally.
    Completed,
    /// The user pressed a skip key.
    Skipped,
}

/// Wait for the specified duration with an animated countdown display.
///
/// Shows a spinner with countdown like: "⠋ Next block in 3s... [n: next | p: pause]"
///
/// # Arguments
///
/// * `delay_secs` - Duration to wait in seconds (supports fractional values)
/// * `is_terminal` - Whether stdout is a terminal (no display if false)
///
/// # Returns
///
/// Returns `CountdownResult::Skipped` if the user pressed a skip key,
/// or `CountdownResult::Completed` if the countdown finished naturally.
///
/// # Behavior
///
/// - If `delay_secs <= 0.0`, returns immediately with `Completed`
/// - If not a terminal, waits without displaying anything
/// - Otherwise, shows animated countdown and clears when done
/// - User can press 'n' or Space to skip to the next block
/// - User can press 'p' to pause/resume the countdown
pub fn wait_with_countdown(delay_secs: f64, is_terminal: bool) -> CountdownResult {
    if delay_secs <= 0.0 {
        return CountdownResult::Completed;
    }

    if !is_terminal {
        // Non-terminal: just sleep without display or input handling
        std::thread::sleep(Duration::from_secs_f64(delay_secs));
        return CountdownResult::Completed;
    }

    run_countdown(delay_secs)
}

/// Run the countdown animation with skip key support.
fn run_countdown(delay_secs: f64) -> CountdownResult {
    // Flush any pending output before starting countdown
    let _ = std::io::stdout().flush();

    // Enable raw mode for keypress detection
    let raw_mode_enabled = terminal::enable_raw_mode().is_ok();

    let result = run_countdown_loop(delay_secs);

    // Disable raw mode before clearing line and returning
    if raw_mode_enabled {
        let _ = terminal::disable_raw_mode();
    }

    // Clear the countdown line
    clear_line();

    result
}

/// Key action result from checking input.
enum KeyAction {
    None,
    Skip,
    TogglePause,
}

/// State for the countdown loop.
struct CountdownState {
    remaining: Duration,
    paused: bool,
    frame: usize,
}

impl CountdownState {
    fn new(delay_secs: f64) -> Self {
        Self {
            remaining: Duration::from_secs_f64(delay_secs),
            paused: false,
            frame: 0,
        }
    }
}

/// The main countdown loop that handles animation and key events.
fn run_countdown_loop(delay_secs: f64) -> CountdownResult {
    let mut state = CountdownState::new(delay_secs);
    let mut stdout = std::io::stdout();
    let mut last_tick = Instant::now();

    loop {
        // Check for key actions
        match check_for_key_action() {
            KeyAction::Skip => return CountdownResult::Skipped,
            KeyAction::TogglePause => state.paused = !state.paused,
            KeyAction::None => {}
        }

        // Update remaining time if not paused
        if !state.paused {
            let now = Instant::now();
            let elapsed = now.duration_since(last_tick);
            last_tick = now;

            if elapsed >= state.remaining {
                return CountdownResult::Completed;
            }
            state.remaining -= elapsed;
        } else {
            // Reset last_tick when paused so we don't count paused time
            last_tick = Instant::now();
        }

        // Render the display
        render_countdown_line(&mut stdout, &state);

        state.frame += 1;
        std::thread::sleep(SPINNER_INTERVAL);
    }
}

/// Render the countdown line (running or paused state).
fn render_countdown_line(stdout: &mut std::io::Stdout, state: &CountdownState) {
    let line = if state.paused {
        format!("{CR}{CLEAR_EOL}{YELLOW}{PAUSE_CHAR}{RESET} {DIM}Paused [p: play | n: next]{RESET}")
    } else {
        let remaining_secs = state.remaining.as_secs_f64().ceil() as u64;
        let spinner_char = SPINNER_CHARS[state.frame % SPINNER_CHARS.len()];
        format!(
            "{CR}{CLEAR_EOL}{CYAN}{}{RESET} {DIM}Next block in {}s... [n: next | p: pause]{RESET}",
            spinner_char, remaining_secs
        )
    };

    let _ = write!(stdout, "{}", line);
    let _ = stdout.flush();
}

/// Check for key actions (skip or pause toggle).
fn check_for_key_action() -> KeyAction {
    // Poll with zero timeout - non-blocking check
    if event::poll(Duration::ZERO).unwrap_or(false) {
        if let Ok(Event::Key(key_event)) = event::read() {
            return classify_key(key_event);
        }
    }
    KeyAction::None
}

/// Classify a key event as a specific action.
///
/// Key mappings:
/// - 'n', 'N', Space, Ctrl+C: Skip to next block
/// - 'p', 'P': Toggle pause/play
fn classify_key(key_event: KeyEvent) -> KeyAction {
    // Only handle key press events, not release or repeat
    if key_event.kind != crossterm::event::KeyEventKind::Press {
        return KeyAction::None;
    }

    match key_event.code {
        // Skip keys
        KeyCode::Char('n' | 'N' | ' ') if key_event.modifiers.is_empty() => KeyAction::Skip,
        KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            KeyAction::Skip
        }
        // Pause toggle
        KeyCode::Char('p' | 'P') if key_event.modifiers.is_empty() => KeyAction::TogglePause,
        _ => KeyAction::None,
    }
}

/// Check if a key event is a skip key (for use in tests).
#[cfg(test)]
fn is_skip_key(key_event: KeyEvent) -> bool {
    matches!(classify_key(key_event), KeyAction::Skip)
}

/// Check if a key event is a pause toggle key (for use in tests).
#[cfg(test)]
fn is_pause_key(key_event: KeyEvent) -> bool {
    matches!(classify_key(key_event), KeyAction::TogglePause)
}

/// Clear the current line.
fn clear_line() {
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "{CR}{CLEAR_EOL}");
    let _ = stdout.flush();
}

/// Apply delay between output elements with optional countdown.
///
/// This is a convenience wrapper that handles the terminal detection
/// and delay application in one call.
///
/// # Arguments
///
/// * `delay_secs` - Optional delay duration in seconds
///
/// # Returns
///
/// `CountdownResult::Skipped` if the user pressed a skip key,
/// `CountdownResult::Completed` if the delay completed or was `None`/`<= 0.0`.
pub fn apply_delay_with_countdown(delay_secs: Option<f64>) -> CountdownResult {
    match delay_secs {
        Some(secs) if secs > 0.0 => {
            let is_terminal = std::io::stdout().is_terminal();
            wait_with_countdown(secs, is_terminal)
        }
        _ => CountdownResult::Completed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wait_with_countdown_zero_delay() {
        // Should return immediately
        let start = Instant::now();
        let result = wait_with_countdown(0.0, false);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
        assert_eq!(result, CountdownResult::Completed);
    }

    #[test]
    fn test_wait_with_countdown_negative_delay() {
        // Should return immediately
        let start = Instant::now();
        let result = wait_with_countdown(-1.0, false);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
        assert_eq!(result, CountdownResult::Completed);
    }

    #[test]
    fn test_wait_with_countdown_non_terminal() {
        // Should wait without display
        let start = Instant::now();
        let result = wait_with_countdown(0.1, false);
        let elapsed = start.elapsed();
        // Should have waited at least 100ms
        assert!(elapsed >= Duration::from_millis(90));
        // But not too long
        assert!(elapsed < Duration::from_millis(200));
        assert_eq!(result, CountdownResult::Completed);
    }

    #[test]
    fn test_apply_delay_with_countdown_none() {
        // Should return immediately
        let start = Instant::now();
        let result = apply_delay_with_countdown(None);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
        assert_eq!(result, CountdownResult::Completed);
    }

    #[test]
    fn test_apply_delay_with_countdown_zero() {
        // Should return immediately
        let start = Instant::now();
        let result = apply_delay_with_countdown(Some(0.0));
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
        assert_eq!(result, CountdownResult::Completed);
    }

    #[test]
    fn test_spinner_chars_available() {
        // Verify spinner chars are available
        assert_eq!(SPINNER_CHARS.len(), 10);
    }

    #[test]
    fn test_countdown_result_enum() {
        // Verify enum variants exist and are distinct
        assert_ne!(CountdownResult::Completed, CountdownResult::Skipped);
    }

    #[test]
    fn test_is_skip_key_n() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty());
        assert!(is_skip_key(key));
    }

    #[test]
    fn test_is_skip_key_n_uppercase() {
        let key = KeyEvent::new(KeyCode::Char('N'), KeyModifiers::empty());
        assert!(is_skip_key(key));
    }

    #[test]
    fn test_is_skip_key_space() {
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty());
        assert!(is_skip_key(key));
    }

    #[test]
    fn test_is_skip_key_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(is_skip_key(key));
    }

    #[test]
    fn test_is_not_skip_key_other() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty());
        assert!(!is_skip_key(key));
    }

    #[test]
    fn test_is_not_skip_key_n_with_modifier() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert!(!is_skip_key(key));
    }

    #[test]
    fn test_is_pause_key_p() {
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::empty());
        assert!(is_pause_key(key));
    }

    #[test]
    fn test_is_pause_key_p_uppercase() {
        let key = KeyEvent::new(KeyCode::Char('P'), KeyModifiers::empty());
        assert!(is_pause_key(key));
    }

    #[test]
    fn test_is_not_pause_key_with_modifier() {
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(!is_pause_key(key));
    }

    #[test]
    fn test_is_not_pause_key_other() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty());
        assert!(!is_pause_key(key));
    }

    #[test]
    fn test_p_is_not_skip_key() {
        // Ensure 'p' doesn't trigger skip (it's pause)
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::empty());
        assert!(!is_skip_key(key));
    }

    #[test]
    fn test_n_is_not_pause_key() {
        // Ensure 'n' doesn't trigger pause (it's skip)
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty());
        assert!(!is_pause_key(key));
    }

    #[test]
    fn test_countdown_state_new() {
        let state = CountdownState::new(5.0);
        assert_eq!(state.remaining.as_secs_f64(), 5.0);
        assert!(!state.paused);
        assert_eq!(state.frame, 0);
    }
}
