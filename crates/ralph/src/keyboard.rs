//! Keyboard input handling for interactive terminal controls.
//!
//! This module provides infrastructure for handling keyboard input during
//! subprocess execution. It includes:
//!
//! - [`RawModeGuard`]: RAII guard for terminal raw mode management
//! - [`RunKeyAction`]: Actions available during the main run loop
//! - Non-blocking keyboard polling functions
//!
//! # Raw Mode Lifecycle
//!
//! Terminal raw mode must be carefully managed to avoid corrupting subprocess output:
//!
//! 1. **When raw mode is enabled**: During spinner display and keyboard polling
//! 2. **When raw mode is disabled**: Before any subprocess stdout/stderr is written
//! 3. **Panic safety**: [`RawModeGuard`] ensures cleanup via Drop implementation
//! 4. **Non-terminal handling**: Raw mode is never enabled when stdout is not a TTY
//!
//! # Example
//!
//! ```no_run
//! use ralph::keyboard::{RawModeGuard, check_for_run_action, RunKeyAction};
//!
//! // Enable raw mode with RAII guard
//! let _guard = RawModeGuard::new();
//!
//! // Poll for keyboard input (non-blocking)
//! match check_for_run_action() {
//!     RunKeyAction::SoftStop => println!("Finishing after this iteration..."),
//!     RunKeyAction::HardStop => println!("Stopping immediately..."),
//!     RunKeyAction::Pause => println!("Paused"),
//!     RunKeyAction::Interrupt => println!("Interrupted!"),
//!     RunKeyAction::None => {} // No key pressed
//! }
//! // Raw mode automatically disabled when guard is dropped
//! ```

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use std::time::Duration;

/// RAII guard for terminal raw mode.
///
/// Ensures raw mode is disabled when dropped, providing panic safety.
/// Raw mode must be disabled before any subprocess execution to prevent
/// stdout corruption.
///
/// # Example
///
/// ```no_run
/// use ralph::keyboard::RawModeGuard;
///
/// {
///     let _guard = RawModeGuard::new();
///     // Raw mode is now active - can read individual keypresses
///     // ...
/// } // Raw mode automatically disabled here
/// ```
#[must_use = "guard must be held to keep raw mode active; dropping disables raw mode"]
pub struct RawModeGuard {
    enabled: bool,
}

impl RawModeGuard {
    /// Enable raw mode and return a guard that disables it on drop.
    ///
    /// If raw mode fails to enable (e.g., not a terminal), the guard
    /// will still be created but will not attempt to disable on drop.
    pub fn new() -> Self {
        Self {
            enabled: terminal::enable_raw_mode().is_ok(),
        }
    }

    /// Check if raw mode was successfully enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl Default for RawModeGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.enabled {
            let _ = terminal::disable_raw_mode();
        }
    }
}

/// Keyboard actions available during the main run loop.
///
/// These actions are detected via non-blocking keyboard polling and
/// allow users to control execution without interrupting the subprocess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKeyAction {
    /// No key was pressed.
    None,
    /// Soft stop: finish current iteration then exit (s key).
    SoftStop,
    /// Hard stop: immediately halt and save paused state (S key).
    HardStop,
    /// Toggle pause/resume (p key).
    Pause,
    /// Interrupt: immediately halt (Ctrl+C).
    Interrupt,
}

/// Check for keyboard actions during the run loop (non-blocking).
///
/// Uses zero-timeout polling for non-blocking input detection. This allows
/// subprocess execution to continue while checking for user input.
///
/// # Requirements
///
/// Raw mode must be active for proper keypress detection. If raw mode is
/// not active, this function will not detect individual keypresses.
///
/// # Returns
///
/// - [`RunKeyAction::SoftStop`] if 's' is pressed
/// - [`RunKeyAction::HardStop`] if 'S' (shift+s) is pressed
/// - [`RunKeyAction::Pause`] if 'p' or 'P' is pressed
/// - [`RunKeyAction::Interrupt`] if Ctrl+C is pressed
/// - [`RunKeyAction::None`] if no key was pressed or on error
///
/// # Key Bindings
///
/// | Key | Action | Description |
/// |-----|--------|-------------|
/// | `s` | SoftStop | Finish current iteration, then exit |
/// | `S` | HardStop | Immediately halt and save paused state |
/// | `p`, `P` | Pause | Toggle pause/resume |
/// | `Ctrl+C` | Interrupt | Immediately halt subprocess |
pub fn check_for_run_action() -> RunKeyAction {
    // Poll with zero timeout - non-blocking check
    if event::poll(Duration::ZERO).unwrap_or(false) {
        if let Ok(Event::Key(key_event)) = event::read() {
            return classify_run_key(key_event);
        }
    }
    RunKeyAction::None
}

/// Classify a key event as a run loop action.
///
/// Only handles key press events (not release or repeat).
fn classify_run_key(key_event: KeyEvent) -> RunKeyAction {
    // Only handle key press events, not release or repeat
    if key_event.kind != KeyEventKind::Press {
        return RunKeyAction::None;
    }

    match key_event.code {
        // Soft stop: 's' without modifiers
        KeyCode::Char('s') if key_event.modifiers.is_empty() => RunKeyAction::SoftStop,
        // Hard stop: 'S' (shift+s) - note: KeyCode::Char already gives uppercase
        KeyCode::Char('S') if key_event.modifiers.is_empty() => RunKeyAction::HardStop,
        // Pause toggle: 'p' or 'P' without other modifiers
        KeyCode::Char('p' | 'P') if key_event.modifiers.is_empty() => RunKeyAction::Pause,
        // Interrupt: Ctrl+C
        KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            RunKeyAction::Interrupt
        }
        _ => RunKeyAction::None,
    }
}

/// Keyboard actions available during replay countdown.
///
/// These actions are specific to the replay countdown display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountdownKeyAction {
    /// No key was pressed.
    None,
    /// Skip to next block (n, N, Space, Ctrl+C).
    Skip,
    /// Toggle pause/resume (p, P).
    TogglePause,
}

/// Check for keyboard actions during replay countdown (non-blocking).
///
/// Uses zero-timeout polling for non-blocking input detection.
///
/// # Requirements
///
/// Raw mode must be active for proper keypress detection.
///
/// # Key Bindings
///
/// | Key | Action | Description |
/// |-----|--------|-------------|
/// | `n`, `N`, `Space` | Skip | Skip to next block |
/// | `Ctrl+C` | Skip | Skip to next block |
/// | `p`, `P` | TogglePause | Toggle pause/resume |
pub fn check_for_countdown_action() -> CountdownKeyAction {
    // Poll with zero timeout - non-blocking check
    if event::poll(Duration::ZERO).unwrap_or(false) {
        if let Ok(Event::Key(key_event)) = event::read() {
            return classify_countdown_key(key_event);
        }
    }
    CountdownKeyAction::None
}

/// Classify a key event as a countdown action.
fn classify_countdown_key(key_event: KeyEvent) -> CountdownKeyAction {
    // Only handle key press events, not release or repeat
    if key_event.kind != KeyEventKind::Press {
        return CountdownKeyAction::None;
    }

    match key_event.code {
        // Skip keys
        KeyCode::Char('n' | 'N' | ' ') if key_event.modifiers.is_empty() => {
            CountdownKeyAction::Skip
        }
        KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            CountdownKeyAction::Skip
        }
        // Pause toggle
        KeyCode::Char('p' | 'P') if key_event.modifiers.is_empty() => {
            CountdownKeyAction::TogglePause
        }
        _ => CountdownKeyAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RawModeGuard tests

    #[test]
    fn test_raw_mode_guard_tracks_enabled_state() {
        // Test that guard tracks the enabled state correctly
        // Note: We can't test actual raw mode in unit tests (requires terminal)
        // but we can verify the guard struct behavior
        let guard = RawModeGuard { enabled: true };
        assert!(guard.is_enabled());

        let guard2 = RawModeGuard { enabled: false };
        assert!(!guard2.is_enabled());
    }

    #[test]
    fn test_raw_mode_guard_default() {
        // Default should attempt to enable raw mode
        // In test environment this will likely fail (no terminal)
        let guard = RawModeGuard::default();
        // Just verify it doesn't panic
        let _ = guard.is_enabled();
    }

    // Run key action tests

    #[test]
    fn test_classify_run_key_soft_stop() {
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty());
        assert_eq!(classify_run_key(key), RunKeyAction::SoftStop);
    }

    #[test]
    fn test_classify_run_key_hard_stop() {
        let key = KeyEvent::new(KeyCode::Char('S'), KeyModifiers::empty());
        assert_eq!(classify_run_key(key), RunKeyAction::HardStop);
    }

    #[test]
    fn test_classify_run_key_pause_lowercase() {
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::empty());
        assert_eq!(classify_run_key(key), RunKeyAction::Pause);
    }

    #[test]
    fn test_classify_run_key_pause_uppercase() {
        let key = KeyEvent::new(KeyCode::Char('P'), KeyModifiers::empty());
        assert_eq!(classify_run_key(key), RunKeyAction::Pause);
    }

    #[test]
    fn test_classify_run_key_ctrl_c_interrupt() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(classify_run_key(key), RunKeyAction::Interrupt);
    }

    #[test]
    fn test_classify_run_key_c_without_modifier() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::empty());
        assert_eq!(classify_run_key(key), RunKeyAction::None);
    }

    #[test]
    fn test_classify_run_key_s_with_modifier_ignored() {
        // 's' with Ctrl should not trigger soft stop
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(classify_run_key(key), RunKeyAction::None);
    }

    #[test]
    fn test_classify_run_key_unknown() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty());
        assert_eq!(classify_run_key(key), RunKeyAction::None);
    }

    #[test]
    fn test_classify_run_key_release_ignored() {
        // Key release events should be ignored
        let mut key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty());
        key.kind = KeyEventKind::Release;
        assert_eq!(classify_run_key(key), RunKeyAction::None);
    }

    // Countdown key action tests

    #[test]
    fn test_classify_countdown_key_skip_n() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty());
        assert_eq!(classify_countdown_key(key), CountdownKeyAction::Skip);
    }

    #[test]
    fn test_classify_countdown_key_skip_n_uppercase() {
        let key = KeyEvent::new(KeyCode::Char('N'), KeyModifiers::empty());
        assert_eq!(classify_countdown_key(key), CountdownKeyAction::Skip);
    }

    #[test]
    fn test_classify_countdown_key_skip_space() {
        let key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty());
        assert_eq!(classify_countdown_key(key), CountdownKeyAction::Skip);
    }

    #[test]
    fn test_classify_countdown_key_skip_ctrl_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(classify_countdown_key(key), CountdownKeyAction::Skip);
    }

    #[test]
    fn test_classify_countdown_key_pause() {
        let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::empty());
        assert_eq!(classify_countdown_key(key), CountdownKeyAction::TogglePause);
    }

    #[test]
    fn test_classify_countdown_key_unknown() {
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty());
        assert_eq!(classify_countdown_key(key), CountdownKeyAction::None);
    }

    #[test]
    fn test_classify_countdown_key_n_with_modifier_ignored() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL);
        assert_eq!(classify_countdown_key(key), CountdownKeyAction::None);
    }

    // Enum equality tests

    #[test]
    fn test_run_key_action_enum_variants() {
        assert_ne!(RunKeyAction::None, RunKeyAction::SoftStop);
        assert_ne!(RunKeyAction::SoftStop, RunKeyAction::HardStop);
        assert_ne!(RunKeyAction::HardStop, RunKeyAction::Pause);
    }

    #[test]
    fn test_countdown_key_action_enum_variants() {
        assert_ne!(CountdownKeyAction::None, CountdownKeyAction::Skip);
        assert_ne!(CountdownKeyAction::Skip, CountdownKeyAction::TogglePause);
    }
}
