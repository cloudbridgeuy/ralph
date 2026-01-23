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
    extract_response_text, write_iteration_log, Chunk, IterationError, IterationLog, LogMetadata,
    LogToolCall,
};
use crate::session::{finalize_session, initialize_session, SessionError};
use crate::signal;
use crate::spinner::SpinnerSessionInfo;
use crate::startup::{
    display_iteration_header, display_iteration_summary, display_prompt, display_startup_info,
    AttachedFile, IterationHeader, IterationSummary, PromptDisplay, StartupInfo,
};
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, invoke_subprocess_with_timeout, SpinnerSubprocessConfig,
    SubprocessError,
};
use crate::summarize::{maybe_summarize_progress, SummarizeConfig};
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
    /// The prompt passed to Claude CLI (after placeholder substitution).
    /// Stored in session metadata for replay purposes.
    pub prompt: String,
    /// Completion marker string to detect in output.
    pub completion_marker: String,
    /// Context file paths.
    pub context_paths: ContextPaths,
    /// Maximum number of attempts when subprocess fails.
    /// A value of 3 means up to 4 total attempts (1 initial + 3 recovery attempts).
    pub max_attempts: usize,
    /// Starting iteration number (for session continuation after failure recovery).
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
    /// Whether user provided additional prompt instructions.
    pub custom_additional_prompt: bool,
    /// Configuration for progress file auto-summarization.
    pub summarize_config: SummarizeConfig,
    /// Configuration for verbose tool output.
    pub verbose_tools_config: VerboseToolsConfig,
    /// Whether to display the prompt before iterations begin.
    /// When true, the prompt is shown before Iteration 1.
    pub show_prompt: bool,
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
    /// Final count of pending stories.
    pub final_pending_stories: usize,
    /// Total cost across all iterations (USD).
    pub total_cost_usd: Option<f64>,
    /// Total duration across all iterations (milliseconds).
    pub total_duration_ms: Option<u64>,
    /// Total input tokens across all iterations.
    pub total_input_tokens: Option<u64>,
    /// Total output tokens across all iterations.
    pub total_output_tokens: Option<u64>,
}

/// Configuration for a single subprocess invocation with failure recovery.
///
/// Groups parameters needed by `invoke_with_failure_recovery` to keep
/// the function signature under 5 arguments.
#[derive(Debug, Clone)]
struct InvocationConfig<'a> {
    /// Full command to invoke the LLM.
    command: &'a str,
    /// Maximum number of recovery attempts after initial failure.
    max_attempts: usize,
    /// Timeout in seconds for each subprocess invocation.
    timeout_secs: u64,
    /// Current iteration number (for logging context).
    iteration: usize,
    /// Theme configuration for syntax highlighting.
    theme_config: Option<&'a ThemeConfig>,
    /// Accumulated time from previous iterations (for spinner display).
    session_elapsed_ms: u64,
    /// Configuration for verbose tool output.
    verbose_tools_config: &'a VerboseToolsConfig,
    /// Session slug for spinner display.
    session_slug: &'a str,
    /// Maximum iterations for spinner display.
    max_iterations: usize,
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

    /// Run was interrupted by a signal (SIGINT/SIGTERM)
    #[error("Run interrupted by signal")]
    Interrupted {
        /// Session slug for finalization
        session_slug: String,
        /// Number of iterations completed before interrupt
        iterations_completed: usize,
        /// Partial result from interrupted subprocess (if interrupt occurred during execution).
        /// Boxed to avoid making the error enum too large since StreamingSubprocessResult
        /// contains vectors of output blocks and other data.
        partial_result: Option<Box<crate::subprocess::StreamingSubprocessResult>>,
        /// Number of pending stories at iteration start (for partial iteration log)
        pending_before: Option<usize>,
    },
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
            // Fall back to initializing a new session (no prompt for fallback case)
            initialize_session(None, &project_path, None)?
        }
    } else {
        // First run - initialize a new session with the prompt
        initialize_session(
            config.slug.as_deref(),
            &project_path,
            Some(config.prompt.clone()),
        )?
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
            custom_additional_prompt: config.custom_additional_prompt,
            session_dir: session_dir.clone(),
        };
        display_startup_info(&startup_info);

        // Display the prompt if enabled
        if config.show_prompt {
            // Build attached files from context paths
            let attached_files = vec![
                AttachedFile::new(config.context_paths.design.clone()),
                AttachedFile::new(config.context_paths.prd.clone()),
                AttachedFile::new(config.context_paths.progress.clone()),
            ];
            let prompt_display = PromptDisplay {
                prompt: &config.prompt,
                attached_files,
            };
            display_prompt(&prompt_display);
        }
    }

    // 6. Execute iteration loop
    let mut iterations_completed = 0;
    let mut completion_reason = None;
    let mut final_pending_stories = prd_analysis.pending_count;

    // Accumulators for aggregated metrics
    let mut total_cost_usd: Option<f64> = None;
    let mut total_duration_ms: Option<u64> = None;
    let mut total_input_tokens: Option<u64> = None;
    let mut total_output_tokens: Option<u64> = None;

    // Calculate iteration numbers: if continuing a session, offset by starting_iteration
    let iteration_offset = config.starting_iteration;

    // Calculate remaining iterations: respect the total limit across session continuations
    // If max_iterations is 5 and we've already done 2 (iteration_offset), only run 3 more
    let remaining_iterations = max_iterations.saturating_sub(iteration_offset);

    for relative_iteration in 1..=remaining_iterations {
        // Check for interrupt at the start of each iteration
        if signal::is_interrupted() {
            return Err(RunError::Interrupted {
                session_slug,
                iterations_completed,
                partial_result: None,
                pending_before: None,
            });
        }

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

        // Invoke LLM subprocess with retry logic, timeout, spinner, and stream processing
        // Pass accumulated session time and session info for spinner display
        let invocation_config = InvocationConfig {
            command: &config.command,
            max_attempts: config.max_attempts,
            timeout_secs: config.timeout_secs,
            iteration,
            theme_config: config.theme_config.as_ref(),
            session_elapsed_ms: total_duration_ms.unwrap_or(0),
            verbose_tools_config: &config.verbose_tools_config,
            session_slug: &session_slug,
            max_iterations,
        };
        let result = match invoke_with_failure_recovery(&invocation_config) {
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
            Err(RunError::Interrupted { partial_result, .. }) => {
                // Interrupt during subprocess - propagate with session context and pending_before
                // The caller will write partial iteration log and finalize session
                return Err(RunError::Interrupted {
                    session_slug,
                    iterations_completed,
                    partial_result,
                    pending_before: Some(pending_before),
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

        // Extract response text from output blocks
        let response = extract_response_text(&result.stream_result.output_blocks);

        // Write iteration log with extracted metadata, tool calls, typed chunks, and output blocks
        let iteration_log = IterationLog {
            sequence: iteration as u32,
            started_at: chrono::Utc::now(),
            completed_at: chrono::Utc::now(),
            exit_code: result.exit_code,
            pending_before,
            pending_after: 0, // Will be updated below
            prompt: None,     // Run command doesn't track prompt per iteration
            response,
            metadata: if metadata.is_empty() {
                None
            } else {
                Some(metadata)
            },
            tool_calls,
            chunks,
            output_blocks: result.stream_result.output_blocks.clone(),
        };

        write_iteration_log(&session_dir, &iteration_log)?;

        // Check for interrupt after writing iteration log (preserves partial data)
        if signal::is_interrupted() {
            // Return interrupt error - iteration log already saved
            return Err(RunError::Interrupted {
                session_slug,
                iterations_completed: relative_iteration, // Include this completed iteration
                partial_result: None,
                pending_before: None,
            });
        }

        // Attempt progress file summarization if configured
        if config.summarize_config.should_summarize() {
            if let Err(e) =
                maybe_summarize_progress(&config.context_paths.progress, &config.summarize_config)
            {
                // Log warning but don't fail the run
                eprintln!("Warning: Failed to summarize progress file: {}", e);
            }
        }

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

        // Accumulate metrics for final summary
        if let Some(cost) = result.stream_result.costs.cost_usd {
            total_cost_usd = Some(total_cost_usd.unwrap_or(0.0) + cost);
        }
        if let Some(duration) = result.stream_result.costs.duration_ms {
            total_duration_ms = Some(total_duration_ms.unwrap_or(0) + duration);
        }
        if let Some(tokens) = input_tokens {
            total_input_tokens = Some(total_input_tokens.unwrap_or(0) + tokens);
        }
        if let Some(tokens) = output_tokens {
            total_output_tokens = Some(total_output_tokens.unwrap_or(0) + tokens);
        }

        // Post-iteration check: re-read PRD
        let prd_after = read_prd_file(&config.context_paths.prd)?;
        let prd_changed = has_prd_changed(&prd_snapshot, &prd_after);

        // Count pending stories after iteration
        let pending_after = count_pending_stories(&prd_after)?;
        final_pending_stories = pending_after;

        // Update iteration log with pending_after
        let mut updated_log = iteration_log.clone();
        updated_log.pending_after = pending_after;
        write_iteration_log(&session_dir, &updated_log)?;

        // Check completion conditions (use raw_text for completion marker detection)
        // This check happens BEFORE the PRD unchanged check because:
        // 1. If completion marker is found, user explicitly wants to stop
        // 2. If all stories are complete, we should exit successfully
        if let Some(reason) = check_completion(
            pending_after,
            &result.stream_result.raw_text,
            &config.completion_marker,
        ) {
            completion_reason = Some(reason);
            iterations_completed = relative_iteration;
            break;
        }

        // Error if PRD unchanged (stuck state) - only check if no completion condition met
        // This ensures that completion markers take precedence over the "stuck" detection
        if !prd_changed {
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

    // Return result with aggregated metrics
    Ok(RunResult {
        slug: session_slug,
        iterations_completed,
        completion_reason,
        final_pending_stories,
        total_cost_usd,
        total_duration_ms,
        total_input_tokens,
        total_output_tokens,
    })
}

/// Read PRD file content.
fn read_prd_file(path: &PathBuf) -> Result<String, RunError> {
    fs::read_to_string(path).map_err(|e| RunError::ReadPrd {
        path: path.display().to_string(),
        source: e,
    })
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
/// Returns the `StreamingSubprocessResult` on success (exit code 0).
fn invoke_with_failure_recovery(
    config: &InvocationConfig,
) -> Result<crate::subprocess::StreamingSubprocessResult, RunError> {
    let total_attempts = config.max_attempts + 1; // max_attempts of 3 means 4 total attempts (1 initial + 3 recovery)

    for attempt in 1..=total_attempts {
        // Use spinner-aware subprocess if theme config provided, otherwise use default
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
                invoke_subprocess_with_spinner_config(&spinner_config)
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
