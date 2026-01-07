//! Iteration loop execution for the run command (Imperative Shell).
//!
//! This module orchestrates the main iteration loop that drives LLM-based
//! development. It integrates all Layer 1 features: context initialization,
//! session management, subprocess invocation, PRD parsing, completion detection,
//! and git diff capture.

use crate::git::capture_and_write_diff;
use crate::highlight::ThemeConfig;
use crate::init::{initialize_context_files, InitError};
use crate::iteration::{
    write_iteration_log, Chunk, IterationError, IterationLog, LogMetadata, LogToolCall,
};
use crate::session::{finalize_session, initialize_session, SessionError};
use crate::startup::{
    display_iteration_header, display_iteration_summary, display_startup_info, IterationHeader,
    IterationSummary, StartupInfo,
};
use crate::subprocess::{
    invoke_subprocess_with_theme, invoke_subprocess_with_timeout, SubprocessError,
};
use ralph_core::completion::{check_completion, CompletionReason};
use ralph_core::context::ContextPaths;
use ralph_core::prd::{count_pending_stories, has_prd_changed, parse_prd, PrdError};
use ralph_core::session::SessionOutcome;
use std::fs;
use std::path::PathBuf;

#[cfg(test)]
mod tests;

/// Configuration for running the iteration loop.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Maximum number of iterations to run (defaults to pending story count).
    pub max_iterations: Option<usize>,
    /// Session slug (auto-generated if None).
    pub slug: Option<String>,
    /// Full command to invoke the LLM (with prompt already substituted).
    pub command: String,
    /// Completion marker string to detect in output.
    pub completion_marker: String,
    /// Context file paths.
    pub context_paths: ContextPaths,
    /// Number of automatic retries on subprocess failure.
    pub retry_count: usize,
    /// Starting iteration number (for session continuation after retries).
    /// When continuing a session, this indicates how many iterations were
    /// already completed in previous run attempts.
    pub starting_iteration: usize,
    /// Timeout in seconds for LLM subprocess (default: 600 = 10 minutes).
    /// If exceeded, the subprocess is killed and treated as a failure.
    pub timeout_secs: u64,
    /// Configuration for syntax highlighting themes.
    /// If None, uses environment variables or default theme.
    pub theme_config: Option<ThemeConfig>,
    /// Whether user provided custom PRD path (for startup display).
    pub custom_prd_path: Option<PathBuf>,
    /// Whether user provided custom design path (for startup display).
    pub custom_design_path: Option<PathBuf>,
    /// Whether user provided custom progress path (for startup display).
    pub custom_progress_path: Option<PathBuf>,
    /// Whether user provided custom command template.
    pub custom_command: bool,
    /// Whether user provided custom prompt.
    pub custom_prompt: bool,
    /// Whether user provided custom completion marker.
    pub custom_completion_marker: bool,
}

/// Result of running the iteration loop.
#[derive(Debug)]
pub struct RunResult {
    /// The session slug used.
    pub slug: String,
    /// Number of iterations completed.
    pub iterations_completed: usize,
    /// Reason for completion (if completed successfully).
    pub completion_reason: Option<CompletionReason>,
}

/// Error type for run operations.
#[derive(thiserror::Error, Debug)]
pub enum RunError {
    /// Context file initialization failed
    #[error("Failed to initialize context files: {0}")]
    Init(#[from] InitError),

    /// Session initialization failed
    #[error("Failed to initialize session: {0}")]
    Session(#[from] SessionError),

    /// PRD parsing failed
    #[error("Failed to parse PRD: {0}")]
    Prd(#[from] PrdError),

    /// Subprocess invocation failed (spawn error, signal, etc.)
    #[error("Failed to invoke subprocess: {0}")]
    Subprocess(#[from] SubprocessError),

    /// LLM subprocess exited with non-zero code after exhausting retries
    #[error("LLM subprocess failed with exit code {exit_code} after {attempts} attempt(s)")]
    SubprocessFailed {
        exit_code: i32,
        attempts: usize,
        /// Raw accumulated text from assistant events (for debugging)
        raw_text: String,
        stderr: String,
        /// Session slug for later finalization
        session_slug: String,
        /// Number of iterations completed before failure
        iterations_completed: usize,
    },

    /// LLM subprocess timed out after exhausting retries
    #[error("LLM subprocess timed out after {timeout_secs} seconds ({attempts} attempt(s))")]
    SubprocessTimedOut {
        timeout_secs: u64,
        attempts: usize,
        /// Partial raw text captured before timeout
        raw_text: String,
        stderr: String,
        /// Session slug for later finalization
        session_slug: String,
        /// Number of iterations completed before timeout
        iterations_completed: usize,
    },

    /// Iteration log writing failed
    #[error("Failed to write iteration log: {0}")]
    Iteration(#[from] IterationError),

    /// PRD file read failed
    #[error("Failed to read PRD file at {path}: {source}")]
    ReadPrd {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// PRD unchanged after iteration (stuck state)
    #[error("PRD unchanged after iteration. LLM may be stuck. Check progress.txt for notes.")]
    PrdUnchanged,

    /// No pending stories to process
    #[error("No pending stories in PRD. All work is complete.")]
    NoPendingStories,
}

/// Execute the main iteration loop.
///
/// This is the capstone function that integrates all Layer 1 features:
/// 1. Pre-check: exit if zero pending stories
/// 2. Snapshot PRD content
/// 3. Invoke LLM subprocess
/// 4. Write iteration log
/// 5. Capture git diff
/// 6. Post-check: error if PRD unchanged
/// 7. Post-check: exit if zero pending or marker found
/// 8. Increment iteration counter and loop
///
/// # Arguments
///
/// * `config` - Configuration for the run
///
/// # Returns
///
/// Returns a `RunResult` with session information and completion reason.
///
/// # Errors
///
/// Returns `RunError` if:
/// - Context file initialization fails
/// - Session initialization fails
/// - PRD parsing fails
/// - Subprocess invocation fails
/// - PRD is unchanged after an iteration (stuck state)
pub fn run(config: RunConfig) -> Result<RunResult, RunError> {
    // 1. Initialize context files (touch missing design/progress, verify PRD exists)
    initialize_context_files(&config.context_paths)?;

    // 2. Read PRD and analyze stories
    let prd_content = read_prd_file(&config.context_paths.prd)?;
    let prd_analysis = parse_prd(&prd_content)?;

    // Pre-check: exit if zero pending stories
    if prd_analysis.pending_count == 0 {
        return Err(RunError::NoPendingStories);
    }

    // 3. Determine max iterations (use provided or default to pending count)
    let max_iterations = config.max_iterations.unwrap_or(prd_analysis.pending_count);
    let iterations_from_arg = config.max_iterations.is_some();

    // 4. Initialize or continue session
    let project_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (session_slug, session_dir) = if config.starting_iteration > 0 {
        // Continuing an existing session - reuse the slug and get the session directory
        if let Some(slug) = config.slug.as_deref() {
            let dir = crate::session::session_dir(slug);
            (slug.to_string(), dir)
        } else {
            // This shouldn't happen - if starting_iteration > 0, we should have a slug
            // Fall back to initializing a new session
            initialize_session(None, &project_path)?
        }
    } else {
        // First run - initialize a new session
        initialize_session(config.slug.as_deref(), &project_path)?
    };

    // 5. Display startup information (only on first run, not retries)
    if config.starting_iteration == 0 {
        let startup_info = StartupInfo {
            slug: session_slug.clone(),
            total_stories: prd_analysis.total_stories,
            pending_stories: prd_analysis.pending_count,
            completed_stories: prd_analysis.completed_count,
            max_iterations,
            iterations_from_arg,
            custom_prd_path: config.custom_prd_path.clone(),
            custom_design_path: config.custom_design_path.clone(),
            custom_progress_path: config.custom_progress_path.clone(),
            custom_command: config.custom_command,
            custom_prompt: config.custom_prompt,
            custom_completion_marker: config.custom_completion_marker,
            session_dir: session_dir.clone(),
        };
        display_startup_info(&startup_info);
    }

    // 6. Execute iteration loop
    let mut iterations_completed = 0;
    let mut completion_reason = None;

    // Calculate iteration numbers: if continuing a session, offset by starting_iteration
    let iteration_offset = config.starting_iteration;

    // Calculate remaining iterations: respect the total limit across session continuations
    // If max_iterations is 5 and we've already done 2 (iteration_offset), only run 3 more
    let remaining_iterations = max_iterations.saturating_sub(iteration_offset);

    for relative_iteration in 1..=remaining_iterations {
        // The actual iteration number includes any completed iterations from prior retries
        let iteration = iteration_offset + relative_iteration;
        // Pre-iteration check: re-read PRD and count pending
        let prd_before = read_prd_file(&config.context_paths.prd)?;
        let pending_before = count_pending_stories(&prd_before)?;

        // Early exit if no pending stories
        if pending_before == 0 {
            completion_reason = Some(CompletionReason::AllStoriesComplete);
            break;
        }

        // Display iteration header before starting work
        // max_iterations is the total user-specified limit (not adjusted for offset)
        let header = IterationHeader {
            iteration,
            max_iterations: Some(max_iterations),
            pending_stories: pending_before,
        };
        display_iteration_header(&header);

        // Snapshot PRD content for change detection
        let prd_snapshot = prd_before.clone();

        // Invoke LLM subprocess with retry logic, timeout, and stream processing
        let result = match invoke_with_retries(
            &config.command,
            config.retry_count,
            config.timeout_secs,
            iteration,
            config.theme_config.as_ref(),
        ) {
            Ok(r) => r,
            Err(RunError::SubprocessFailed {
                exit_code,
                attempts,
                raw_text,
                stderr,
                ..
            }) => {
                // Subprocess failed - return error with session info for caller to finalize
                // The caller will determine whether to mark as "failed" or "aborted"
                return Err(RunError::SubprocessFailed {
                    exit_code,
                    attempts,
                    raw_text,
                    stderr,
                    session_slug,
                    iterations_completed,
                });
            }
            Err(RunError::SubprocessTimedOut {
                timeout_secs,
                attempts,
                raw_text,
                stderr,
                ..
            }) => {
                // Subprocess timed out - return error with session info for caller to finalize
                return Err(RunError::SubprocessTimedOut {
                    timeout_secs,
                    attempts,
                    raw_text,
                    stderr,
                    session_slug,
                    iterations_completed,
                });
            }
            Err(e) => return Err(e),
        };

        // Build metadata from stream processing result
        let metadata = LogMetadata::from_extracted(
            result.stream_result.metadata.clone(),
            result.stream_result.costs.clone(),
        );

        // Build tool calls from stream processing result
        let tool_calls = LogToolCall::from_interactions(&result.stream_result.tool_interactions);

        // Convert parsed chunks to iteration log chunks
        let chunks = Chunk::from_parsed_chunks(&result.stream_result.chunks);

        // Write iteration log with extracted metadata, tool calls, and typed chunks
        let iteration_log = IterationLog {
            sequence: iteration as u32,
            started_at: chrono::Utc::now(),
            completed_at: chrono::Utc::now(),
            exit_code: result.exit_code,
            pending_before,
            pending_after: 0, // Will be updated below
            metadata: if metadata.is_empty() {
                None
            } else {
                Some(metadata)
            },
            tool_calls,
            chunks,
        };

        write_iteration_log(&session_dir, &iteration_log)?;

        // Capture git diff
        let diff_path = session_dir.join(format!("iteration-{}.diff", iteration));
        if let Err(e) = capture_and_write_diff(&diff_path) {
            eprintln!("Warning: Failed to capture git diff: {}", e);
        }

        // Display iteration summary with cost, duration, and token usage
        let (input_tokens, output_tokens) = result
            .stream_result
            .costs
            .usage
            .as_ref()
            .map(|u| (Some(u.input_tokens), Some(u.output_tokens)))
            .unwrap_or((None, None));

        let summary = IterationSummary {
            iteration,
            cost_usd: result.stream_result.costs.cost_usd,
            duration_ms: result.stream_result.costs.duration_ms,
            model: result.stream_result.metadata.model.clone(),
            input_tokens,
            output_tokens,
        };
        display_iteration_summary(&summary);

        // Post-iteration check: re-read PRD
        let prd_after = read_prd_file(&config.context_paths.prd)?;

        // Error if PRD unchanged (stuck state)
        if !has_prd_changed(&prd_snapshot, &prd_after) {
            // Finalize session as failed before returning error
            // Use total iterations including prior retries
            let total_so_far = iteration_offset + relative_iteration;
            if let Err(e) =
                finalize_session(&session_slug, total_so_far as u32, SessionOutcome::Failed)
            {
                eprintln!("Warning: Failed to finalize session: {}", e);
            }
            return Err(RunError::PrdUnchanged);
        }

        // Count pending stories after iteration
        let pending_after = count_pending_stories(&prd_after)?;

        // Update iteration log with pending_after
        let mut updated_log = iteration_log.clone();
        updated_log.pending_after = pending_after;
        write_iteration_log(&session_dir, &updated_log)?;

        // Check completion conditions (use raw_text for completion marker detection)
        if let Some(reason) = check_completion(
            pending_after,
            &result.stream_result.raw_text,
            &config.completion_marker,
        ) {
            completion_reason = Some(reason);
            iterations_completed = relative_iteration;
            break;
        }

        iterations_completed = relative_iteration;
    }

    // Finalize session as completed (use total iterations including prior retries)
    let total_iterations = iteration_offset + iterations_completed;
    if let Err(e) = finalize_session(
        &session_slug,
        total_iterations as u32,
        SessionOutcome::Completed,
    ) {
        eprintln!("Warning: Failed to finalize session: {}", e);
    }

    // Return result
    Ok(RunResult {
        slug: session_slug,
        iterations_completed,
        completion_reason,
    })
}

/// Read PRD file content.
fn read_prd_file(path: &PathBuf) -> Result<String, RunError> {
    fs::read_to_string(path).map_err(|e| RunError::ReadPrd {
        path: path.display().to_string(),
        source: e,
    })
}

/// Invoke subprocess with automatic retries on failure and timeout support.
///
/// This function wraps `invoke_subprocess_with_timeout` or `invoke_subprocess_with_theme`
/// with retry logic:
/// - On non-zero exit code, prints raw text/stderr and retries
/// - On timeout, prints partial output and retries
/// - Prints attempt number for each retry
/// - Returns error with captured output if all retries exhausted
///
/// # Arguments
///
/// * `command` - The command to execute
/// * `retry_count` - Number of retries (0 means run once with no retries)
/// * `timeout_secs` - Timeout in seconds for each subprocess invocation
/// * `iteration` - Current iteration number (for logging context)
/// * `theme_config` - Optional theme configuration for syntax highlighting
///
/// # Returns
///
/// Returns the `StreamingSubprocessResult` on success (exit code 0).
fn invoke_with_retries(
    command: &str,
    retry_count: usize,
    timeout_secs: u64,
    iteration: usize,
    theme_config: Option<&ThemeConfig>,
) -> Result<crate::subprocess::StreamingSubprocessResult, RunError> {
    let max_attempts = retry_count + 1; // retry_count of 3 means 4 total attempts

    for attempt in 1..=max_attempts {
        // Use theme-aware subprocess if config provided, otherwise use default
        let result = match theme_config {
            Some(config) => {
                match invoke_subprocess_with_theme(command, timeout_secs, config.clone()) {
                    Ok(r) => Ok(r),
                    Err(e) => Err(e),
                }
            }
            None => invoke_subprocess_with_timeout(command, timeout_secs),
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
                    iteration, ts, attempt, max_attempts
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

                if attempt < max_attempts {
                    eprintln!("\nRetrying...\n");
                    continue;
                } else {
                    // All retries exhausted due to timeout
                    return Err(RunError::SubprocessTimedOut {
                        timeout_secs: ts,
                        attempts: max_attempts,
                        raw_text: partial_result.stream_result.raw_text,
                        stderr: partial_result.stderr,
                        session_slug: String::new(),
                        iterations_completed: 0,
                    });
                }
            }
            Err(e) => return Err(e.into()),
        };

        if result.exit_code == 0 {
            return Ok(result);
        }

        // Non-zero exit code - handle retry
        eprintln!(
            "\n[Iteration {}] LLM subprocess failed with exit code {} (attempt {}/{})",
            iteration, result.exit_code, attempt, max_attempts
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

        if attempt < max_attempts {
            eprintln!("\nRetrying...\n");
        } else {
            // All retries exhausted - return error with placeholder session info
            // The caller will fill in the actual session_slug and iterations_completed
            return Err(RunError::SubprocessFailed {
                exit_code: result.exit_code,
                attempts: max_attempts,
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
