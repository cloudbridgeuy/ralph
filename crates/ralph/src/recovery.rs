//! Subprocess failure recovery functionality.
//!
//! Shared recovery logic for subprocess invocations with automatic retry.
//! Used by both the run command and strategy implementations.

use crate::highlight::ThemeConfig;
use crate::keyboard::RunKeyAction;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, invoke_subprocess_with_timeout, SpinnerSubprocessConfig,
    StreamingSubprocessResult, SubprocessError,
};

/// Configuration for a single subprocess invocation with failure recovery.
///
/// Groups parameters needed by `invoke_with_failure_recovery` to keep
/// the function signature under 5 arguments.
pub struct InvocationConfig<'a> {
    /// Full command to invoke the LLM.
    pub command: &'a str,
    /// Maximum number of recovery attempts after initial failure.
    pub max_attempts: usize,
    /// Timeout in seconds for each subprocess invocation.
    pub timeout_secs: u64,
    /// Current iteration number (for logging context).
    pub iteration: usize,
    /// Theme configuration for syntax highlighting.
    pub theme_config: Option<&'a ThemeConfig>,
    /// Accumulated time from previous iterations (for spinner display).
    pub session_elapsed_ms: u64,
    /// Configuration for verbose tool output.
    pub verbose_tools_config: &'a VerboseToolsConfig,
    /// Session slug for spinner display.
    pub session_slug: &'a str,
    /// Maximum iterations for spinner display.
    pub max_iterations: usize,
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

/// Error type for recovery operations.
///
/// Contains only the error variants that can originate from subprocess
/// invocation and failure recovery. Callers wrap these into their own
/// domain-specific error types as needed.
#[derive(thiserror::Error, Debug)]
pub enum RecoveryError {
    /// LLM subprocess exited with non-zero code after exhausting retries
    #[error("LLM subprocess failed with exit code {exit_code} after {attempts} attempt(s)")]
    SubprocessFailed {
        exit_code: i32,
        attempts: usize,
        raw_text: String,
        stderr: String,
    },

    /// LLM subprocess timed out after exhausting retries
    #[error("LLM subprocess timed out after {timeout_secs} seconds ({attempts} attempt(s))")]
    SubprocessTimedOut {
        timeout_secs: u64,
        attempts: usize,
        raw_text: String,
        stderr: String,
    },

    /// Run was interrupted by a signal (SIGINT/SIGTERM)
    #[error("Subprocess interrupted by signal")]
    Interrupted {
        partial_result: Option<Box<StreamingSubprocessResult>>,
    },

    /// Run was hard-stopped by user (S key)
    #[error("Subprocess hard-stopped by user")]
    HardStop {
        partial_result: Option<Box<StreamingSubprocessResult>>,
    },

    /// Subprocess invocation failed (spawn error, signal, etc.)
    #[error("Failed to invoke subprocess: {0}")]
    Subprocess(#[from] SubprocessError),
}

impl RecoveryError {
    /// Extract partial result from interrupt/hard-stop errors.
    pub fn partial_result(&self) -> Option<&StreamingSubprocessResult> {
        match self {
            Self::Interrupted { partial_result } | Self::HardStop { partial_result } => {
                partial_result.as_deref()
            }
            _ => None,
        }
    }

    /// Take the partial result, leaving None in its place.
    pub fn take_partial_result(&mut self) -> Option<Box<StreamingSubprocessResult>> {
        match self {
            Self::Interrupted { partial_result } | Self::HardStop { partial_result } => {
                partial_result.take()
            }
            _ => None,
        }
    }

    /// Whether this is a hard stop that should save paused state.
    pub fn is_hard_stop(&self) -> bool {
        matches!(self, Self::HardStop { .. })
    }
}

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

/// Invoke subprocess with automatic failure recovery and timeout support.
///
/// Wraps subprocess invocation with retry logic:
/// - On non-zero exit code, prints raw text/stderr and re-attempts
/// - On timeout, prints partial output and re-attempts
/// - Returns error with captured output if all attempts exhausted
pub fn invoke_with_failure_recovery(
    config: &InvocationConfig,
) -> Result<RecoveryOutcome, RecoveryError> {
    let total_attempts = config.max_attempts + 1;

    let mut detected_key_action: Option<RunKeyAction> = None;

    for attempt in 1..=total_attempts {
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

        if key_action.is_some() {
            detected_key_action = key_action;
        }

        let result = match subprocess_result {
            Ok(r) => r,
            Err(SubprocessError::Timeout {
                timeout_secs: ts,
                partial_result,
            }) => {
                eprintln!(
                    "\n[Iteration {}] LLM subprocess timed out after {} seconds (attempt {}/{})",
                    config.iteration, ts, attempt, total_attempts
                );
                print_captured_output("partial output", &partial_result.stream_result.raw_text);
                print_captured_output("stderr", &partial_result.stderr);

                if attempt < total_attempts {
                    eprintln!("\nRe-attempting...\n");
                    continue;
                } else {
                    return Err(RecoveryError::SubprocessTimedOut {
                        timeout_secs: ts,
                        attempts: total_attempts,
                        raw_text: partial_result.stream_result.raw_text,
                        stderr: partial_result.stderr,
                    });
                }
            }
            Err(SubprocessError::Interrupted { partial_result }) => {
                return Err(RecoveryError::Interrupted {
                    partial_result: Some(partial_result),
                });
            }
            Err(SubprocessError::HardStop { partial_result }) => {
                return Err(RecoveryError::HardStop {
                    partial_result: Some(partial_result),
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

        eprintln!(
            "\n[Iteration {}] LLM subprocess failed with exit code {} (attempt {}/{})",
            config.iteration, result.exit_code, attempt, total_attempts
        );
        print_captured_output("captured output", &result.stream_result.raw_text);
        print_captured_output("stderr", &result.stderr);

        if attempt < total_attempts {
            eprintln!("\nRe-attempting...\n");
        } else {
            return Err(RecoveryError::SubprocessFailed {
                exit_code: result.exit_code,
                attempts: total_attempts,
                raw_text: result.stream_result.raw_text,
                stderr: result.stderr,
            });
        }
    }

    unreachable!("Loop should have returned or errored")
}
