//! Subprocess failure recovery functionality.
//!
//! This module handles automatic retry logic for failed subprocess invocations.

use super::{InvocationConfig, RunError};
use crate::keyboard::RunKeyAction;
use crate::spinner::SpinnerSessionInfo;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, invoke_subprocess_with_timeout, SpinnerSubprocessConfig,
    StreamingSubprocessResult, SubprocessError,
};

/// Print captured output with a header, ensuring trailing newline.
fn print_captured_output(header: &str, content: &str) {
    if content.is_empty() {
        return;
    }
    eprintln!("\n--- {} ---", header);
    eprint!("{}", content);
    if !content.ends_with('\n') {
        eprintln!();
    }
}

/// Result of subprocess invocation with failure recovery.
///
/// Bundles the subprocess result with any keyboard action detected during execution.
pub struct RecoveryOutcome {
    /// The subprocess execution result.
    pub subprocess_result: StreamingSubprocessResult,
    /// Keyboard action detected during execution (if any).
    pub key_action: Option<RunKeyAction>,
}

/// Invoke subprocess with automatic failure recovery and timeout support.
///
/// This function wraps `invoke_subprocess_with_spinner_config` or `invoke_subprocess_with_timeout`
/// with failure recovery logic:
/// - On non-zero exit code, prints raw text/stderr and re-attempts
/// - On timeout, prints partial output and re-attempts
/// - Prints attempt number for each attempt
/// - Returns error with captured output if all attempts exhausted
///
/// # Arguments
///
/// * `config` - Configuration for the invocation including command, max_attempts,
///   timeout_secs, iteration number, theme_config, and session_elapsed_ms
///
/// # Returns
///
/// Returns a `RecoveryOutcome` containing the subprocess result and any detected key action.
pub fn invoke_with_failure_recovery(
    config: &InvocationConfig,
) -> Result<RecoveryOutcome, RunError> {
    let total_attempts = config.max_attempts + 1; // max_attempts of 3 means 4 total attempts (1 initial + 3 recovery)

    // Track the most recent key action detected across attempts
    let mut detected_key_action: Option<RunKeyAction> = None;

    for attempt in 1..=total_attempts {
        // Use spinner-aware subprocess if theme config provided
        let (subprocess_result, key_action) = match config.theme_config {
            Some(theme) => {
                let spinner_config = SpinnerSubprocessConfig {
                    command: config.command.to_string(),
                    timeout_secs: config.timeout_secs,
                    theme_config: theme.clone(),
                    session_elapsed_ms: config.session_elapsed_ms,
                    verbose_tools: config.verbose_tools_config.clone(),
                    session_info: SpinnerSessionInfo::new(
                        config.session_slug.to_string(),
                        config.iteration,
                        config.max_iterations,
                    ),
                };
                let outcome = invoke_subprocess_with_spinner_config(&spinner_config);
                (outcome.subprocess_result, outcome.key_action)
            }
            None => (
                invoke_subprocess_with_timeout(
                    config.command,
                    config.timeout_secs,
                    config.verbose_tools_config.clone(),
                ),
                None,
            ),
        };

        // Capture any detected key action
        if key_action.is_some() {
            detected_key_action = key_action;
        }

        let result = match subprocess_result {
            Ok(r) => r,
            Err(SubprocessError::Timeout {
                timeout_secs: ts,
                partial_result,
            }) => {
                // Timeout occurred
                eprintln!(
                    "\n[Iteration {}] LLM subprocess timed out after {} seconds (attempt {}/{})",
                    config.iteration, ts, attempt, total_attempts
                );

                // Print partial output from timed out attempt
                print_captured_output("partial output", &partial_result.stream_result.raw_text);
                print_captured_output("stderr", &partial_result.stderr);

                if attempt < total_attempts {
                    eprintln!("\nRe-attempting...\n");
                    continue;
                } else {
                    // All attempts exhausted due to timeout
                    return Err(RunError::SubprocessTimedOut {
                        timeout_secs: ts,
                        attempts: total_attempts,
                        raw_text: partial_result.stream_result.raw_text,
                        stderr: partial_result.stderr,
                        session_slug: String::new(),
                        iterations_completed: 0,
                    });
                }
            }
            Err(SubprocessError::Interrupted { partial_result }) => {
                // Interrupt occurred - don't retry, propagate immediately with partial result
                // The caller will handle session finalization and write partial iteration log
                return Err(RunError::Interrupted {
                    session_slug: String::new(),
                    iterations_completed: 0,
                    partial_result: Some(partial_result),
                    pending_before: None,
                });
            }
            Err(e) => return Err(e.into()),
        };

        if result.exit_code == 0 {
            return Ok(RecoveryOutcome {
                subprocess_result: result,
                key_action: detected_key_action,
            });
        }

        // Non-zero exit code - handle failure recovery
        eprintln!(
            "\n[Iteration {}] LLM subprocess failed with exit code {} (attempt {}/{})",
            config.iteration, result.exit_code, attempt, total_attempts
        );

        // Print captured output from failed attempt
        print_captured_output("captured output", &result.stream_result.raw_text);
        print_captured_output("stderr", &result.stderr);

        if attempt < total_attempts {
            eprintln!("\nRe-attempting...\n");
        } else {
            // All attempts exhausted - return error with placeholder session info
            // The caller will fill in the actual session_slug and iterations_completed
            return Err(RunError::SubprocessFailed {
                exit_code: result.exit_code,
                attempts: total_attempts,
                raw_text: result.stream_result.raw_text,
                stderr: result.stderr,
                session_slug: String::new(),
                iterations_completed: 0,
            });
        }
    }

    // This should never be reached due to the return in the loop
    unreachable!("Loop should have returned or errored")
}
