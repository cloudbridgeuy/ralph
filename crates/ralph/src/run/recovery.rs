//! Subprocess failure recovery and resume functionality.
//!
//! This module handles:
//! - Automatic retry logic for failed subprocess invocations
//! - Resume functionality after hard stops
//! - Building continuation prompts for interrupted iterations

use super::{InvocationConfig, RunError};
use crate::spinner::SpinnerSessionInfo;
use crate::subprocess::{
    invoke_subprocess_with_keyboard, invoke_subprocess_with_timeout, SpinnerSubprocessConfig,
    SubprocessError,
};

/// Continuation prompt for resuming from a hard stop.
///
/// This message is appended to the command when resuming after the user pressed 'S'
/// and then 'p' to continue. It instructs the LLM to continue from where it left off.
/// Note: "session" here refers to the Claude CLI conversation session (maintained via
/// --session-id), not the ralph session.
const RESUME_CONTINUATION_PROMPT: &str =
    "Continue working from where you left off. The previous attempt was interrupted.";

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
        // Use spinner-aware subprocess with keyboard monitoring if theme config provided
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
                // Use keyboard-aware subprocess to detect soft stop
                invoke_subprocess_with_keyboard(&spinner_config, config.keyboard_monitor)
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
            Err(SubprocessError::HardStopped { partial_result }) => {
                // Hard stop occurred - don't retry, propagate immediately with partial result
                // The caller will handle the paused state and potential resume
                return Err(RunError::HardStopped {
                    session_slug: String::new(),
                    iterations_completed: 0,
                    partial_result: Some(partial_result),
                    pending_before: None,
                    total_cost_usd: None,
                    total_duration_ms: None,
                    total_input_tokens: None,
                    total_output_tokens: None,
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

/// Invoke subprocess to resume an interrupted iteration.
///
/// This function modifies the command to include a continuation prompt that tells
/// Claude to continue from where it left off. The same `--session-id` is maintained
/// since we're still in the same ralph session.
///
/// # Arguments
///
/// * `config` - Configuration for the invocation (same as normal iteration)
///
/// # Returns
///
/// Returns the `StreamingSubprocessResult` on success, or `RunError` on failure.
pub fn invoke_resume_iteration(
    config: &InvocationConfig,
) -> Result<crate::subprocess::StreamingSubprocessResult, RunError> {
    // Build a modified command with the continuation prompt
    // We need to inject the continuation prompt into the existing command
    // The command format is: claude ... -p 'original prompt'
    // We want to append: '\n\n<continuation message>'

    let resume_command = build_resume_command(config.command, RESUME_CONTINUATION_PROMPT);

    // Create a modified config with the resume command
    let resume_config = InvocationConfig {
        command: &resume_command,
        max_attempts: config.max_attempts,
        timeout_secs: config.timeout_secs,
        iteration: config.iteration,
        theme_config: config.theme_config,
        session_elapsed_ms: config.session_elapsed_ms,
        verbose_tools_config: config.verbose_tools_config,
        session_slug: config.session_slug,
        max_iterations: config.max_iterations,
        keyboard_monitor: config.keyboard_monitor,
    };

    invoke_with_failure_recovery(&resume_config)
}

/// Build a resume command by appending a continuation prompt.
///
/// The command is expected to end with `-p 'some prompt'`. This function appends
/// the continuation message to that prompt.
///
/// # Arguments
///
/// * `original_command` - The original command string
/// * `continuation_prompt` - The message to append (e.g., "Continue working...")
///
/// # Returns
///
/// The modified command with the continuation prompt appended.
fn build_resume_command(original_command: &str, continuation_prompt: &str) -> String {
    // The command ends with -p 'prompt content'
    // We need to find the last single quote and inject our continuation before it
    // Handle edge cases: nested quotes, escaped quotes, etc.

    // Simple approach: find the last `'` and insert before it
    if let Some(last_quote_idx) = original_command.rfind('\'') {
        let (before, after) = original_command.split_at(last_quote_idx);
        format!("{}\n\n{}{}", before, continuation_prompt, after)
    } else {
        // Fallback: just append the continuation as a separate argument
        // This shouldn't happen with the default command template
        format!("{} -p '{}'", original_command, continuation_prompt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_resume_command_appends_continuation() {
        let original = "claude --session-id abc -p 'Do the work'";
        let result = build_resume_command(original, "Continue from where you left off.");

        assert!(result.contains("Do the work"));
        assert!(result.contains("Continue from where you left off."));
        // The continuation should be before the closing quote
        assert!(result.ends_with('\''));
    }

    #[test]
    fn test_build_resume_command_no_quotes_fallback() {
        let original = "claude --session-id abc";
        let result = build_resume_command(original, "Continue.");

        // Should fallback to appending a new -p argument
        assert!(result.contains("-p 'Continue.'"));
    }

    #[test]
    fn test_build_resume_command_preserves_session_id() {
        let original = "claude --session-id my-uuid -p 'Work on task'";
        let result = build_resume_command(original, "Resume.");

        assert!(result.contains("--session-id my-uuid"));
    }
}
