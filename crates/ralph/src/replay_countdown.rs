//! Countdown display for replay delays.
//!
//! Provides an animated countdown spinner displayed between output blocks
//! during replay when the `--delay` flag is used. The countdown is
//! non-interactive: it ticks to zero and clears.
//!
//! # Features
//!
//! - Animated spinner with countdown timer
//! - Updates each tick with remaining time
//! - Clears automatically before the next block
//! - No display when delay is `<= 0.0` or stdout is not a terminal
//!
//! # Example
//!
//! ```no_run
//! use ralph::replay_countdown::wait_with_countdown;
//!
//! // Wait 3 seconds with countdown display
//! wait_with_countdown(3.0, true);
//! ```

use crate::ansi::{CLEAR_EOL, CR, CYAN, DIM, RESET};
use crate::spinner::{SPINNER_CHARS, SPINNER_INTERVAL};
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

/// Wait for the specified duration with an animated countdown display.
///
/// Shows a spinner with countdown like: "⠋ Next block in 3s...".
///
/// # Arguments
///
/// * `delay_secs` - Duration to wait in seconds (supports fractional values)
/// * `is_terminal` - Whether stdout is a terminal (no display if false)
///
/// # Behavior
///
/// - If `delay_secs <= 0.0`, returns immediately
/// - If not a terminal, sleeps without displaying anything
/// - Otherwise, shows animated countdown and clears when done
pub fn wait_with_countdown(delay_secs: f64, is_terminal: bool) {
    if delay_secs <= 0.0 {
        return;
    }

    if !is_terminal {
        std::thread::sleep(Duration::from_secs_f64(delay_secs));
        return;
    }

    run_countdown(delay_secs);
}

fn run_countdown(delay_secs: f64) {
    let _ = std::io::stdout().flush();
    run_countdown_loop(delay_secs);
    clear_line();
}

struct CountdownState {
    remaining: Duration,
    frame: usize,
}

impl CountdownState {
    fn new(delay_secs: f64) -> Self {
        Self {
            remaining: Duration::from_secs_f64(delay_secs),
            frame: 0,
        }
    }
}

fn run_countdown_loop(delay_secs: f64) {
    let mut state = CountdownState::new(delay_secs);
    let mut stdout = std::io::stdout();
    let mut last_tick = Instant::now();

    loop {
        let now = Instant::now();
        let elapsed = now.duration_since(last_tick);
        last_tick = now;

        if elapsed >= state.remaining {
            return;
        }
        state.remaining -= elapsed;

        render_countdown_line(&mut stdout, &state);

        state.frame += 1;
        std::thread::sleep(SPINNER_INTERVAL);
    }
}

fn render_countdown_line(stdout: &mut std::io::Stdout, state: &CountdownState) {
    let remaining_secs = state.remaining.as_secs_f64().ceil() as u64;
    let spinner_char = SPINNER_CHARS[state.frame % SPINNER_CHARS.len()];
    let line = format!(
        "{CR}{CLEAR_EOL}{CYAN}{}{RESET} {DIM}Next block in {}s...{RESET}",
        spinner_char, remaining_secs
    );

    let _ = write!(stdout, "{}", line);
    let _ = stdout.flush();
}

fn clear_line() {
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "{CR}{CLEAR_EOL}");
    let _ = stdout.flush();
}

/// Apply delay between output elements with optional countdown.
///
/// Convenience wrapper that handles terminal detection and delay
/// application in one call. No-op when `delay_secs` is `None` or `<= 0.0`.
pub fn apply_delay_with_countdown(delay_secs: Option<f64>) {
    if let Some(secs) = delay_secs {
        if secs > 0.0 {
            let is_terminal = std::io::stdout().is_terminal();
            wait_with_countdown(secs, is_terminal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wait_with_countdown_zero_delay() {
        let start = Instant::now();
        wait_with_countdown(0.0, false);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_wait_with_countdown_negative_delay() {
        let start = Instant::now();
        wait_with_countdown(-1.0, false);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_wait_with_countdown_non_terminal() {
        let start = Instant::now();
        wait_with_countdown(0.1, false);
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(90));
        assert!(elapsed < Duration::from_millis(200));
    }

    #[test]
    fn test_apply_delay_with_countdown_none() {
        let start = Instant::now();
        apply_delay_with_countdown(None);
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_apply_delay_with_countdown_zero() {
        let start = Instant::now();
        apply_delay_with_countdown(Some(0.0));
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_millis(50));
    }

    #[test]
    fn test_spinner_chars_available() {
        assert_eq!(SPINNER_CHARS.len(), 10);
    }

    #[test]
    fn test_countdown_state_new() {
        let state = CountdownState::new(5.0);
        assert_eq!(state.remaining.as_secs_f64(), 5.0);
        assert_eq!(state.frame, 0);
    }
}
