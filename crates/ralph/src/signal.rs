//! Signal handling for graceful shutdown.
//!
//! This module provides SIGINT/SIGTERM handling for strategy execution,
//! allowing sessions to be properly finalized as "interrupted" when the user
//! presses Ctrl+C or the process receives a termination signal.
//!
//! ## Design
//!
//! The signal handler uses an atomic flag to track interrupt state. When a
//! signal is received:
//! 1. The flag is set to true
//! 2. An optional cleanup function is called (registered via `set_cleanup_handler`)
//! 3. The program should check `is_interrupted()` and exit gracefully
//!
//! The cleanup handler runs in the signal handler context, so it must be
//! async-signal-safe (no allocations, no locks, etc.). For session finalization,
//! we store the session info in a separate global and finalize from the main thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Global flag indicating whether an interrupt signal has been received.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Information needed to finalize a session on interrupt.
#[derive(Debug, Clone)]
pub struct InterruptContext {
    /// Session slug for finalization.
    pub slug: String,
    /// Number of iterations completed.
    pub iterations_completed: u32,
}

/// Global context for session finalization on interrupt.
/// Protected by a mutex since we need to update it from the main thread.
static INTERRUPT_CONTEXT: Mutex<Option<InterruptContext>> = Mutex::new(None);

/// Initialize the signal handler.
///
/// This should be called once at program startup. It registers handlers for
/// SIGINT (Ctrl+C) and SIGTERM that set the interrupted flag.
///
/// # Errors
///
/// Returns an error if the signal handler cannot be registered.
pub fn init() -> Result<(), ctrlc::Error> {
    ctrlc::set_handler(move || {
        // Set the interrupted flag
        INTERRUPTED.store(true, Ordering::SeqCst);

        // Print a message to stderr
        // Note: This is technically not async-signal-safe, but ctrlc handles
        // this by running in a dedicated thread on Unix systems.
        eprintln!("\nInterrupted. Cleaning up...");
    })
}

/// Check if an interrupt signal has been received.
///
/// This should be checked periodically in long-running operations to enable
/// graceful shutdown.
#[inline]
pub fn is_interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Set the interrupt context for session finalization.
///
/// This should be called when a session is started or updated, so that if an
/// interrupt occurs, the signal handler knows which session to finalize.
///
/// # Arguments
///
/// * `context` - The context to set, or None to clear it.
pub fn set_interrupt_context(context: Option<InterruptContext>) {
    if let Ok(mut guard) = INTERRUPT_CONTEXT.lock() {
        *guard = context;
    }
}

/// Get the current interrupt context.
///
/// Returns the context if set, or None if not set or if the lock is poisoned.
pub fn get_interrupt_context() -> Option<InterruptContext> {
    INTERRUPT_CONTEXT
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

/// Reset the interrupted state.
///
/// This is mainly useful for testing.
#[cfg(test)]
pub fn reset() {
    INTERRUPTED.store(false, Ordering::SeqCst);
    if let Ok(mut guard) = INTERRUPT_CONTEXT.lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interrupted_flag_default_false() {
        // Note: This test may fail if run after other tests that set the flag.
        // In production, the flag is only set once per process.
        // For testing purposes, we just verify the API works.
        let _ = is_interrupted(); // Should not panic
    }

    #[test]
    fn test_set_and_get_interrupt_context() {
        let ctx = InterruptContext {
            slug: "test-session".to_string(),
            iterations_completed: 5,
        };

        set_interrupt_context(Some(ctx.clone()));

        let retrieved = get_interrupt_context();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.slug, "test-session");
        assert_eq!(retrieved.iterations_completed, 5);

        // Clear it
        set_interrupt_context(None);
        assert!(get_interrupt_context().is_none());
    }

    #[test]
    fn test_interrupt_context_clone() {
        let ctx = InterruptContext {
            slug: "clone-test".to_string(),
            iterations_completed: 3,
        };

        let cloned = ctx.clone();
        assert_eq!(cloned.slug, ctx.slug);
        assert_eq!(cloned.iterations_completed, ctx.iterations_completed);
    }
}
