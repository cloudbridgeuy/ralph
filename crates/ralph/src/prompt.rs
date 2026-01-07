//! Interactive user prompting (Imperative Shell).
//!
//! This module provides functions for prompting users in terminal sessions.
//! Prompts are only shown when stdin is a terminal; non-interactive sessions
//! abort automatically.

use std::io::{self, IsTerminal, Write};

/// User's choice when prompted after an unrecoverable failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    /// Continue recovery - re-attempt the operation.
    Retry,
    /// Abort and exit the program.
    Abort,
}

/// Check if stdin is an interactive terminal.
pub fn is_interactive() -> bool {
    io::stdin().is_terminal()
}

/// Prompt the user for an action after an unrecoverable failure.
///
/// Displays the failure summary and asks the user to choose between
/// retrying or aborting. Only works in interactive terminal sessions.
///
/// # Arguments
///
/// * `failure_summary` - A summary of what failed
///
/// # Returns
///
/// Returns `Some(FailureAction)` if the user made a choice, or `None` if:
/// - stdin is not a terminal (non-interactive mode)
/// - EOF was reached (user pressed Ctrl+D)
/// - An I/O error occurred
///
/// # Example
///
/// ```ignore
/// if let Some(action) = prompt_on_failure("LLM subprocess failed after 4 attempts") {
///     match action {
///         FailureAction::Retry => println!("Continuing recovery..."),
///         FailureAction::Abort => println!("Aborting..."),
///     }
/// } else {
///     // Non-interactive or error - abort automatically
///     println!("Non-interactive mode, aborting...");
/// }
/// ```
pub fn prompt_on_failure(failure_summary: &str) -> Option<FailureAction> {
    // Check if stdin is interactive
    if !is_interactive() {
        return None;
    }

    // Display failure summary
    eprintln!("\n{}", failure_summary);
    eprintln!();

    // Prompt for action
    loop {
        eprint!("What would you like to do? [R]etry / [A]bort: ");
        io::stderr().flush().ok();

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => return None, // EOF (Ctrl+D)
            Ok(_) => {
                let choice = input.trim().to_lowercase();
                match choice.as_str() {
                    "r" | "retry" => return Some(FailureAction::Retry),
                    "a" | "abort" => return Some(FailureAction::Abort),
                    "" => {
                        // Empty input - show prompt again
                        eprintln!("Please enter 'r' to retry or 'a' to abort.");
                    }
                    _ => {
                        eprintln!(
                            "Invalid choice '{}'. Please enter 'r' to retry or 'a' to abort.",
                            choice
                        );
                    }
                }
            }
            Err(_) => return None, // I/O error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failure_action_debug() {
        assert_eq!(format!("{:?}", FailureAction::Retry), "Retry");
        assert_eq!(format!("{:?}", FailureAction::Abort), "Abort");
    }

    #[test]
    fn test_failure_action_eq() {
        assert_eq!(FailureAction::Retry, FailureAction::Retry);
        assert_eq!(FailureAction::Abort, FailureAction::Abort);
        assert_ne!(FailureAction::Retry, FailureAction::Abort);
    }

    #[test]
    fn test_failure_action_clone() {
        let action = FailureAction::Retry;
        let cloned = action;
        assert_eq!(action, cloned);
    }

    // Note: Testing prompt_on_failure() interactively is difficult in unit tests.
    // The function checks is_interactive() which will return false in test runners.
    // Integration tests with PTY would be needed for full coverage.
}
