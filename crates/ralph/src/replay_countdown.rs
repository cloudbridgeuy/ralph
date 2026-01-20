//! Countdown display for replay delays.
//!
//! This module provides an animated countdown spinner that displays between
//! output blocks during replay when the `--delay` flag is used.
//!
//! # Features
//!
//! - Animated spinner with countdown timer
//! - Updates every second with remaining time
//! - Clears automatically before the next block
//! - No display when delay is 0 or stdout is not a terminal
//!
//! # Example
//!
//! ```no_run
//! use ralph::replay_countdown::wait_with_countdown;
//!
//! // Wait 3 seconds with countdown display
//! wait_with_countdown(3.0, true);
//! ```

use crate::spinner::{SPINNER_CHARS, SPINNER_INTERVAL};
use std::io::{IsTerminal, Write};
use std::thread;
use std::time::{Duration, Instant};

/// Wait for the specified duration with an animated countdown display.
///
/// Shows a spinner with countdown like: "⠋ Next block in 3s..."
///
/// # Arguments
///
/// * `delay_secs` - Duration to wait in seconds (supports fractional values)
/// * `is_terminal` - Whether stdout is a terminal (no display if false)
///
/// # Behavior
///
/// - If `delay_secs <= 0.0`, returns immediately
/// - If not a terminal, waits without displaying anything
/// - Otherwise, shows animated countdown and clears when done
pub fn wait_with_countdown(delay_secs: f64, is_terminal: bool) {
    if delay_secs <= 0.0 {
        return;
    }

    if !is_terminal {
        // Non-terminal: just sleep without display
        thread::sleep(Duration::from_secs_f64(delay_secs));
        return;
    }

    run_countdown(delay_secs);
}

/// Run the countdown animation.
fn run_countdown(delay_secs: f64) {
    // Flush any pending output before starting countdown
    let _ = std::io::stdout().flush();

    let start = Instant::now();
    let total_duration = Duration::from_secs_f64(delay_secs);
    let mut frame = 0;
    let mut stdout = std::io::stdout();

    loop {
        let elapsed = start.elapsed();
        if elapsed >= total_duration {
            break;
        }

        let remaining = total_duration - elapsed;
        let remaining_secs = remaining.as_secs_f64().ceil() as u64;

        // Get current spinner character
        let spinner_char = SPINNER_CHARS[frame % SPINNER_CHARS.len()];

        // Build countdown line
        let countdown_line = format!(
            "\r\x1b[36m{}\x1b[0m \x1b[2mNext block in {}s...\x1b[0m",
            spinner_char, remaining_secs
        );

        // Write and flush
        let _ = write!(stdout, "{}", countdown_line);
        let _ = stdout.flush();

        frame += 1;
        thread::sleep(SPINNER_INTERVAL);
    }

    // Clear the countdown line
    clear_line();
}

/// Clear the current line.
fn clear_line() {
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "\r\x1b[K");
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
/// Does nothing if `delay_secs` is `None` or `<= 0.0`.
pub fn apply_delay_with_countdown(delay_secs: Option<f64>) {
    if let Some(secs) = delay_secs {
        let is_terminal = std::io::stdout().is_terminal();
        wait_with_countdown(secs, is_terminal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wait_with_countdown_zero_delay() {
        // Should return immediately
        let start = Instant::now();
        wait_with_countdown(0.0, false);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_wait_with_countdown_negative_delay() {
        // Should return immediately
        let start = Instant::now();
        wait_with_countdown(-1.0, false);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_wait_with_countdown_non_terminal() {
        // Should wait without display
        let start = Instant::now();
        wait_with_countdown(0.1, false);
        let elapsed = start.elapsed();
        // Should have waited at least 100ms
        assert!(elapsed >= Duration::from_millis(90));
        // But not too long
        assert!(elapsed < Duration::from_millis(200));
    }

    #[test]
    fn test_apply_delay_with_countdown_none() {
        // Should return immediately
        let start = Instant::now();
        apply_delay_with_countdown(None);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_apply_delay_with_countdown_zero() {
        // Should return immediately
        let start = Instant::now();
        apply_delay_with_countdown(Some(0.0));
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_spinner_chars_available() {
        // Verify spinner chars are available
        assert_eq!(SPINNER_CHARS.len(), 10);
    }
}
