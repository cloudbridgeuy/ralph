//! Subprocess failure recovery functionality.
//!
//! This module handles automatic retry logic for failed subprocess invocations.

use super::{InvocationConfig, RunError};
use crate::spinner::SpinnerSessionInfo;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, invoke_subprocess_with_timeout, SpinnerSubprocessConfig,
    SubprocessError,
};

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
/// Returns the `StreamingSubprocessResult` on success (exit code 0).
pub fn invoke_with_failure_recovery(
    config: &InvocationConfig,
) -> Result<crate::subprocess::StreamingSubprocessResult, RunError> {
    let total_attempts = config.max_attempts + 1; // max_attempts of 3 means 4 total attempts (1 initial + 3 recovery)

    for attempt in 1..=total_attempts {
        // Use spinner-aware subprocess if theme config provided
        let result = match config.theme_config {
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
                // invoke_subprocess_with_spinner_config returns SpinnerSubprocessOutcome
                // Extract subprocess_result to match expected type
                invoke_subprocess_with_spinner_config(&spinner_config).subprocess_result
            }
            None => invoke_subprocess_with_timeout(
                config.command,
                config.timeout_secs,
                config.verbose_tools_config.clone(),
            ),
        };

        let result = match result {
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
                let raw_text = &partial_result.stream_result.raw_text;
                if !raw_text.is_empty() {
                    eprintln!("\n--- partial output ---");
                    eprint!("{}", raw_text);
                    if !raw_text.ends_with('\n') {
                        eprintln!();
                    }
                }

                if !partial_result.stderr.is_empty() {
                    eprintln!("\n--- stderr ---");
                    eprint!("{}", partial_result.stderr);
                    if !partial_result.stderr.ends_with('\n') {
                        eprintln!();
                    }
                }

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
            return Ok(result);
        }

        // Non-zero exit code - handle failure recovery
        eprintln!(
            "\n[Iteration {}] LLM subprocess failed with exit code {} (attempt {}/{})",
            config.iteration, result.exit_code, attempt, total_attempts
        );

        // Print captured output from failed attempt
        let raw_text = &result.stream_result.raw_text;
        if !raw_text.is_empty() {
            eprintln!("\n--- captured output ---");
            eprint!("{}", raw_text);
            if !raw_text.ends_with('\n') {
                eprintln!();
            }
        }

        if !result.stderr.is_empty() {
            eprintln!("\n--- stderr ---");
            eprint!("{}", result.stderr);
            if !result.stderr.ends_with('\n') {
                eprintln!();
            }
        }

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
