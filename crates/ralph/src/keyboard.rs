//! Non-blocking keyboard input monitoring during subprocess execution.
//!
//! This module provides keyboard event detection for runtime control features:
//! - Soft stop ('s'): Pause after current iteration completes
//! - Hard stop ('S'): Interrupt immediately, can resume later
//! - Resume ('p'): Continue from paused state
//! - Quit ('q'): Exit from paused state
//!
//! # Architecture
//!
//! The keyboard monitor runs on a background thread and sends events through
//! a channel. This allows the main subprocess loop to check for input without
//! blocking, while still receiving events when they occur.
//!
//! # Terminal Mode
//!
//! Raw terminal mode is required for key detection. This module handles:
//! - Enabling raw mode when monitoring starts
//! - Disabling raw mode when monitoring stops
//! - Restoring terminal state on panic (via Drop)
//!
//! # Example
//!
//! ```no_run
//! use ralph::keyboard::{KeyboardMonitor, KeyEvent};
//! use std::time::Duration;
//!
//! // Create and start the monitor
//! let mut monitor = KeyboardMonitor::new();
//! monitor.start();
//!
//! // Check for events (non-blocking)
//! loop {
//!     if let Some(event) = monitor.poll() {
//!         match event {
//!             KeyEvent::SoftStop => println!("Soft stop requested"),
//!             KeyEvent::HardStop => println!("Hard stop requested"),
//!             KeyEvent::Resume => println!("Resume requested"),
//!             KeyEvent::Quit => break,
//!         }
//!     }
//!     // Do other work...
//!     std::thread::sleep(Duration::from_millis(100));
//! }
//!
//! // Stop the monitor (also called automatically on drop)
//! monitor.stop();
//! ```

use crossterm::event::{self, Event, KeyCode, KeyEvent as CrosstermKeyEvent, KeyModifiers};
use crossterm::terminal;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Keyboard events recognized by the runtime control system.
///
/// These events map to the control keys documented in the runtime control feature:
/// - 's' → SoftStop
/// - 'S' (Shift+s) → HardStop
/// - 'p' → Resume (also Play)
/// - 'q' → Quit
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    /// Soft stop: pause after current iteration completes ('s')
    SoftStop,
    /// Hard stop: interrupt subprocess immediately ('S')
    HardStop,
    /// Resume/Play: continue from paused state ('p')
    Resume,
    /// Quit: exit from paused state ('q')
    Quit,
}

/// Configuration for the keyboard monitor.
#[derive(Debug, Clone)]
pub struct KeyboardMonitorConfig {
    /// Polling interval for checking keyboard events.
    /// Shorter intervals are more responsive but use more CPU.
    pub poll_interval: Duration,
}

impl Default for KeyboardMonitorConfig {
    fn default() -> Self {
        Self {
            // 50ms provides good responsiveness without excessive CPU usage
            poll_interval: Duration::from_millis(50),
        }
    }
}

/// Non-blocking keyboard input monitor.
///
/// This monitor runs a background thread that polls for keyboard input
/// and sends recognized events through a channel. The main thread can
/// check for events without blocking using `poll()` or `try_recv()`.
///
/// # Thread Safety
///
/// The monitor uses atomic flags for coordination between the main thread
/// and the background polling thread. Events are communicated through
/// an mpsc channel.
///
/// # Terminal Mode
///
/// When active, the monitor enables raw terminal mode to capture individual
/// keypresses. Raw mode is automatically disabled when the monitor is stopped
/// or dropped.
pub struct KeyboardMonitor {
    /// Flag to signal the polling thread to stop.
    running: Arc<AtomicBool>,
    /// Handle to the background polling thread.
    thread_handle: Option<JoinHandle<()>>,
    /// Receiver end of the event channel.
    event_rx: Option<Receiver<KeyEvent>>,
    /// Whether the monitor is enabled (only works in terminal mode).
    enabled: bool,
    /// Whether raw mode was enabled by this monitor.
    raw_mode_active: bool,
    /// Configuration for the monitor.
    config: KeyboardMonitorConfig,
}

impl Default for KeyboardMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyboardMonitor {
    /// Create a new keyboard monitor.
    ///
    /// The monitor is enabled only when stdout is a terminal. When piped
    /// or in non-interactive mode, all monitor methods become no-ops.
    pub fn new() -> Self {
        Self::with_config(KeyboardMonitorConfig::default())
    }

    /// Create a keyboard monitor with custom configuration.
    pub fn with_config(config: KeyboardMonitorConfig) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
            event_rx: None,
            enabled: std::io::stdout().is_terminal(),
            raw_mode_active: false,
            config,
        }
    }

    /// Create a keyboard monitor with explicit enabled state.
    ///
    /// This is primarily useful for testing.
    pub fn with_enabled(enabled: bool) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            thread_handle: None,
            event_rx: None,
            enabled,
            raw_mode_active: false,
            config: KeyboardMonitorConfig::default(),
        }
    }

    /// Check if the monitor is enabled.
    ///
    /// Returns false when not running in a terminal.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if the monitor is currently running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Start the keyboard monitor.
    ///
    /// This enables raw terminal mode and spawns a background thread to
    /// poll for keyboard events. If the monitor is disabled (non-terminal)
    /// or already running, this is a no-op.
    ///
    /// # Returns
    ///
    /// Returns `true` if the monitor was started successfully, `false` if
    /// it was already running, disabled, or failed to enable raw mode.
    pub fn start(&mut self) -> bool {
        if !self.enabled || self.is_running() {
            return false;
        }

        // Enable raw mode for key detection
        if terminal::enable_raw_mode().is_err() {
            return false;
        }
        self.raw_mode_active = true;

        // Create event channel
        let (tx, rx) = mpsc::channel();
        self.event_rx = Some(rx);

        // Set running flag
        self.running.store(true, Ordering::SeqCst);

        // Clone shared state for the polling thread
        let running = Arc::clone(&self.running);
        let poll_interval = self.config.poll_interval;

        // Spawn polling thread
        let handle = thread::spawn(move || {
            run_keyboard_polling(running, tx, poll_interval);
        });

        self.thread_handle = Some(handle);
        true
    }

    /// Stop the keyboard monitor.
    ///
    /// This signals the background thread to stop, waits for it to finish,
    /// and disables raw terminal mode. If the monitor is not running, this
    /// is a no-op.
    pub fn stop(&mut self) {
        if !self.is_running() {
            // Still need to disable raw mode if it's active
            if self.raw_mode_active {
                let _ = terminal::disable_raw_mode();
                self.raw_mode_active = false;
            }
            return;
        }

        // Signal the thread to stop
        self.running.store(false, Ordering::SeqCst);

        // Wait for the thread to finish
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }

        // Disable raw mode
        if self.raw_mode_active {
            let _ = terminal::disable_raw_mode();
            self.raw_mode_active = false;
        }

        // Clear the channel
        self.event_rx = None;
    }

    /// Poll for a keyboard event without blocking.
    ///
    /// Returns `Some(event)` if an event is available, or `None` if no
    /// event is pending. This method never blocks.
    ///
    /// # Returns
    ///
    /// - `Some(KeyEvent)` if an event was received
    /// - `None` if no event is pending or the monitor is not running
    pub fn poll(&self) -> Option<KeyEvent> {
        let rx = self.event_rx.as_ref()?;

        match rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// Drain all pending keyboard events.
    ///
    /// Returns a vector of all events that have accumulated since the last
    /// poll. This is useful when you want to process multiple events at once
    /// or check if a specific event type is pending.
    pub fn drain(&self) -> Vec<KeyEvent> {
        let Some(rx) = self.event_rx.as_ref() else {
            return Vec::new();
        };

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Check if a specific event type is pending.
    ///
    /// This drains all pending events and checks if any match the given type.
    /// Note: This consumes the events, so they won't be available from `poll()`.
    pub fn has_pending(&self, event_type: KeyEvent) -> bool {
        self.drain().contains(&event_type)
    }
}

impl Drop for KeyboardMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Run the keyboard polling loop.
///
/// This function runs on a background thread and polls for keyboard events,
/// sending recognized events through the channel.
fn run_keyboard_polling(running: Arc<AtomicBool>, tx: Sender<KeyEvent>, poll_interval: Duration) {
    while running.load(Ordering::SeqCst) {
        // Poll with timeout - this handles both waiting and event detection
        // No additional sleep needed since poll() already blocks for the interval
        if event::poll(poll_interval).unwrap_or(false) {
            if let Ok(Event::Key(key_event)) = event::read() {
                if let Some(event) = classify_key_event(key_event) {
                    // Send the event (ignore send errors - receiver may be dropped)
                    let _ = tx.send(event);
                }
            }
        }
    }
}

/// Classify a crossterm key event as a runtime control event.
///
/// Only handles key press events (not release or repeat).
///
/// # Key Mappings
///
/// - 's' (lowercase, no modifiers) → SoftStop
/// - 'S' (uppercase/shift, only SHIFT modifier) → HardStop
/// - 'p' (lowercase, no modifiers) or 'P' (uppercase, only SHIFT) → Resume
/// - 'q' (lowercase, no modifiers) or 'Q' (uppercase, only SHIFT) → Quit
fn classify_key_event(key_event: CrosstermKeyEvent) -> Option<KeyEvent> {
    // Only handle key press events
    if key_event.kind != crossterm::event::KeyEventKind::Press {
        return None;
    }

    match key_event.code {
        // Soft stop: lowercase 's' with no modifiers
        KeyCode::Char('s') if key_event.modifiers.is_empty() => Some(KeyEvent::SoftStop),

        // Hard stop: uppercase 'S' (shift+s) with only SHIFT modifier
        KeyCode::Char('S') if key_event.modifiers == KeyModifiers::SHIFT => {
            Some(KeyEvent::HardStop)
        }

        // Resume: 'p' (lowercase, no modifiers) or 'P' (uppercase, only SHIFT)
        KeyCode::Char('p') if key_event.modifiers.is_empty() => Some(KeyEvent::Resume),
        KeyCode::Char('P') if key_event.modifiers == KeyModifiers::SHIFT => Some(KeyEvent::Resume),

        // Quit: 'q' (lowercase, no modifiers) or 'Q' (uppercase, only SHIFT)
        KeyCode::Char('q') if key_event.modifiers.is_empty() => Some(KeyEvent::Quit),
        KeyCode::Char('Q') if key_event.modifiers == KeyModifiers::SHIFT => Some(KeyEvent::Quit),

        _ => None,
    }
}

/// Restore terminal to normal mode.
///
/// This is a standalone function that can be called from panic handlers
/// or signal handlers to ensure the terminal is restored even if the
/// KeyboardMonitor's Drop implementation doesn't run.
///
/// It's safe to call this multiple times - it will only disable raw mode
/// if it's currently enabled.
pub fn restore_terminal() {
    // disable_raw_mode is idempotent - safe to call even if not in raw mode
    let _ = terminal::disable_raw_mode();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_event_variants() {
        // Verify all variants exist and are distinct
        assert_ne!(KeyEvent::SoftStop, KeyEvent::HardStop);
        assert_ne!(KeyEvent::SoftStop, KeyEvent::Resume);
        assert_ne!(KeyEvent::SoftStop, KeyEvent::Quit);
        assert_ne!(KeyEvent::HardStop, KeyEvent::Resume);
        assert_ne!(KeyEvent::HardStop, KeyEvent::Quit);
        assert_ne!(KeyEvent::Resume, KeyEvent::Quit);
    }

    #[test]
    fn test_key_event_clone() {
        let event = KeyEvent::SoftStop;
        let cloned = event;
        assert_eq!(event, cloned);
    }

    #[test]
    fn test_keyboard_monitor_config_default() {
        let config = KeyboardMonitorConfig::default();
        assert_eq!(config.poll_interval, Duration::from_millis(50));
    }

    #[test]
    fn test_keyboard_monitor_disabled() {
        // Monitor created with disabled flag should not start
        let mut monitor = KeyboardMonitor::with_enabled(false);
        assert!(!monitor.is_enabled());
        assert!(!monitor.start());
        assert!(!monitor.is_running());
    }

    #[test]
    fn test_keyboard_monitor_poll_when_not_running() {
        let monitor = KeyboardMonitor::with_enabled(false);
        assert!(monitor.poll().is_none());
    }

    #[test]
    fn test_keyboard_monitor_drain_when_not_running() {
        let monitor = KeyboardMonitor::with_enabled(false);
        assert!(monitor.drain().is_empty());
    }

    #[test]
    fn test_keyboard_monitor_has_pending_when_not_running() {
        let monitor = KeyboardMonitor::with_enabled(false);
        assert!(!monitor.has_pending(KeyEvent::SoftStop));
    }

    #[test]
    fn test_classify_key_event_soft_stop() {
        let key = CrosstermKeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty());
        assert_eq!(classify_key_event(key), Some(KeyEvent::SoftStop));
    }

    #[test]
    fn test_classify_key_event_hard_stop() {
        let key = CrosstermKeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT);
        assert_eq!(classify_key_event(key), Some(KeyEvent::HardStop));
    }

    #[test]
    fn test_classify_key_event_resume_lowercase() {
        let key = CrosstermKeyEvent::new(KeyCode::Char('p'), KeyModifiers::empty());
        assert_eq!(classify_key_event(key), Some(KeyEvent::Resume));
    }

    #[test]
    fn test_classify_key_event_resume_uppercase() {
        // Uppercase 'P' with SHIFT modifier (how crossterm reports Shift+p)
        let key = CrosstermKeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT);
        assert_eq!(classify_key_event(key), Some(KeyEvent::Resume));
    }

    #[test]
    fn test_classify_key_event_quit_lowercase() {
        let key = CrosstermKeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty());
        assert_eq!(classify_key_event(key), Some(KeyEvent::Quit));
    }

    #[test]
    fn test_classify_key_event_quit_uppercase() {
        // Uppercase 'Q' with SHIFT modifier (how crossterm reports Shift+q)
        let key = CrosstermKeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT);
        assert_eq!(classify_key_event(key), Some(KeyEvent::Quit));
    }

    #[test]
    fn test_classify_key_event_s_with_ctrl() {
        // 's' with Ctrl should not be soft stop
        let key = CrosstermKeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(classify_key_event(key), None);
    }

    #[test]
    fn test_classify_key_event_unrecognized() {
        let key = CrosstermKeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty());
        assert_eq!(classify_key_event(key), None);
    }

    #[test]
    fn test_classify_key_event_non_press() {
        // Key release events should be ignored
        let mut key = CrosstermKeyEvent::new(KeyCode::Char('s'), KeyModifiers::empty());
        key.kind = crossterm::event::KeyEventKind::Release;
        assert_eq!(classify_key_event(key), None);
    }

    #[test]
    fn test_keyboard_monitor_stop_when_not_running() {
        // Should be safe to call stop when not running
        let mut monitor = KeyboardMonitor::with_enabled(false);
        monitor.stop(); // Should not panic
    }

    #[test]
    fn test_keyboard_monitor_double_stop() {
        // Should be safe to call stop multiple times
        let mut monitor = KeyboardMonitor::with_enabled(false);
        monitor.stop();
        monitor.stop(); // Should not panic
    }

    #[test]
    fn test_keyboard_monitor_default() {
        let monitor = KeyboardMonitor::default();
        // In test environment, may or may not be a terminal
        // Just verify it doesn't panic
        assert!(!monitor.is_running());
    }
}
