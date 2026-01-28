//! Iteration loop execution for the run command (Imperative Shell).
//!
//! This module orchestrates the main iteration loop that drives LLM-based
//! development. It integrates all Layer 1 features: context initialization,
//! session management, subprocess invocation, PRD parsing, completion detection,
//! and git diff capture.

mod recovery;

use crate::git::capture_and_write_diff;
use crate::highlight::ThemeConfig;
use crate::init::{verify_prd_exists, InitError};
use crate::iteration::{
    extract_response_text, write_iteration_log, Chunk, IterationError, IterationLog, LogMetadata,
    LogToolCall,
};
use crate::keyboard::RunKeyAction;
use crate::session::{finalize_session, initialize_session, SessionError};
use crate::signal;
use crate::startup::{
    display_iteration_header, display_iteration_summary, display_prompt, display_startup_info,
    AttachedFile, IterationHeader, IterationSummary, PromptDisplay, StartupInfo,
};
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::SubprocessError;
use crate::warn::warn_if_err;
use ralph_core::completion::{check_completion, CompletionReason};
use ralph_core::prd::{count_pending_stories, has_prd_changed, parse_prd, PrdError};
use ralph_core::session::SessionOutcome;
use recovery::{invoke_with_failure_recovery, RecoveryOutcome};
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
    /// Path to the PRD file.
    pub prd_path: PathBuf,
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
    /// Whether user provided custom command template.
    pub custom_command: bool,
    /// Whether user provided custom prompt.
    pub custom_prompt: bool,
    /// Whether user provided custom completion marker.
    pub custom_completion_marker: bool,
    /// Whether user provided additional prompt instructions.
    pub custom_additional_prompt: bool,
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
pub(crate) struct InvocationConfig<'a> {
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
    #[error("PRD unchanged after iteration. LLM may be stuck.")]
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
/// - PRD file doesn't exist
/// - Session initialization fails
/// - PRD parsing fails
/// - Subprocess invocation fails
/// - PRD is unchanged after an iteration (stuck state)
pub fn run(config: RunConfig) -> Result<RunResult, RunError> {
    // 1. Verify PRD exists and analyze stories
    verify_prd_exists(&config.prd_path)?;
    let prd_content = read_prd_file(&config.prd_path)?;
    let prd_analysis = parse_prd(&prd_content)?;

    if prd_analysis.pending_count == 0 {
        return Err(RunError::NoPendingStories);
    }

    // 2. Determine max iterations
    let max_iterations = config.max_iterations.unwrap_or(prd_analysis.pending_count);

    // 3. Initialize or continue session
    let (session_slug, session_dir) = initialize_run_session(&config)?;

    // 4. Display startup information (only on first run)
    if config.starting_iteration == 0 {
        display_startup(
            &config,
            &session_slug,
            &session_dir,
            &prd_analysis,
            max_iterations,
        );
    }

    // 5. Execute iteration loop
    let mut state = IterationState::new(prd_analysis.pending_count);
    let iteration_offset = config.starting_iteration;
    let remaining_iterations = max_iterations.saturating_sub(iteration_offset);

    for relative_iteration in 1..=remaining_iterations {
        // Check for interrupt at the start of each iteration
        if signal::is_interrupted() {
            return Err(RunError::Interrupted {
                session_slug,
                iterations_completed: state.iterations_completed,
                partial_result: None,
                pending_before: None,
            });
        }

        let iteration = iteration_offset + relative_iteration;
        let result = process_single_iteration(
            &config,
            &session_slug,
            &session_dir,
            iteration,
            max_iterations,
            relative_iteration,
            &mut state,
        )?;

        match result {
            SingleIterationResult::Complete(reason) => {
                state.completion_reason = Some(reason);
                state.iterations_completed = relative_iteration;
                break;
            }
            SingleIterationResult::Stuck => {
                return Err(RunError::PrdUnchanged);
            }
            SingleIterationResult::Continue(key_action) => {
                state.iterations_completed = relative_iteration;

                // Handle soft stop: finish after current iteration
                if matches!(key_action, Some(RunKeyAction::SoftStop)) {
                    eprintln!("\nSoft stop requested. Finishing after this iteration.");
                    state.completion_reason = Some(CompletionReason::SoftStop);
                    break;
                }
            }
        }
    }

    // Finalize session as completed
    let total_iterations = iteration_offset + state.iterations_completed;
    warn_if_err(
        finalize_session(
            &session_slug,
            total_iterations as u32,
            SessionOutcome::Completed,
        ),
        "Failed to finalize session",
    );

    Ok(RunResult {
        slug: session_slug,
        iterations_completed: state.iterations_completed,
        completion_reason: state.completion_reason,
        final_pending_stories: state.final_pending_stories,
        total_cost_usd: state.metrics.total_cost_usd,
        total_duration_ms: state.metrics.total_duration_ms,
        total_input_tokens: state.metrics.total_input_tokens,
        total_output_tokens: state.metrics.total_output_tokens,
    })
}

/// Read PRD file content.
fn read_prd_file(path: &PathBuf) -> Result<String, RunError> {
    fs::read_to_string(path).map_err(|e| RunError::ReadPrd {
        path: path.display().to_string(),
        source: e,
    })
}

/// Accumulated metrics across iterations.
#[derive(Debug, Default)]
struct AccumulatedMetrics {
    total_cost_usd: Option<f64>,
    total_duration_ms: Option<u64>,
    total_input_tokens: Option<u64>,
    total_output_tokens: Option<u64>,
}

impl AccumulatedMetrics {
    /// Add metrics from a single iteration result.
    fn add_from_result(&mut self, result: &crate::subprocess::StreamingSubprocessResult) {
        if let Some(cost) = result.stream_result.costs.cost_usd {
            self.total_cost_usd = Some(self.total_cost_usd.unwrap_or(0.0) + cost);
        }
        if let Some(duration) = result.stream_result.costs.duration_ms {
            self.total_duration_ms = Some(self.total_duration_ms.unwrap_or(0) + duration);
        }
        if let Some(ref usage) = result.stream_result.costs.usage {
            self.total_input_tokens =
                Some(self.total_input_tokens.unwrap_or(0) + usage.input_tokens);
            self.total_output_tokens =
                Some(self.total_output_tokens.unwrap_or(0) + usage.output_tokens);
        }
    }

    /// Get elapsed time for spinner display.
    fn elapsed_ms(&self) -> u64 {
        self.total_duration_ms.unwrap_or(0)
    }
}

/// Build an iteration log from subprocess result.
fn build_iteration_log(
    iteration: usize,
    pending_before: usize,
    result: &crate::subprocess::StreamingSubprocessResult,
) -> IterationLog {
    let metadata = LogMetadata::from_extracted(
        result.stream_result.metadata.clone(),
        result.stream_result.costs.clone(),
    );
    let tool_calls = LogToolCall::from_interactions(&result.stream_result.tool_interactions);
    let chunks = Chunk::from_parsed_chunks(&result.stream_result.chunks);
    let response = extract_response_text(&result.stream_result.output_blocks);

    IterationLog {
        sequence: iteration as u32,
        started_at: chrono::Utc::now(),
        completed_at: chrono::Utc::now(),
        exit_code: result.exit_code,
        pending_before,
        pending_after: 0, // Updated later after PRD re-read
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
    }
}

/// Display iteration summary with cost, duration, and token usage.
fn display_iteration_metrics(
    iteration: usize,
    result: &crate::subprocess::StreamingSubprocessResult,
) {
    let usage = result.stream_result.costs.usage.as_ref();
    let summary = IterationSummary {
        iteration,
        cost_usd: result.stream_result.costs.cost_usd,
        duration_ms: result.stream_result.costs.duration_ms,
        model: result.stream_result.metadata.model.clone(),
        input_tokens: usage.map(|u| u.input_tokens),
        output_tokens: usage.map(|u| u.output_tokens),
    };
    display_iteration_summary(&summary);
}

/// Handle subprocess invocation with error context enrichment.
///
/// Wraps `invoke_with_failure_recovery` to add session context to errors.
fn handle_subprocess_invocation(
    config: &InvocationConfig,
    session_slug: &str,
    iterations_completed: usize,
    pending_before: usize,
) -> Result<RecoveryOutcome, RunError> {
    match invoke_with_failure_recovery(config) {
        Ok(outcome) => Ok(outcome),
        Err(RunError::SubprocessFailed {
            exit_code,
            attempts,
            raw_text,
            stderr,
            ..
        }) => Err(RunError::SubprocessFailed {
            exit_code,
            attempts,
            raw_text,
            stderr,
            session_slug: session_slug.to_string(),
            iterations_completed,
        }),
        Err(RunError::SubprocessTimedOut {
            timeout_secs,
            attempts,
            raw_text,
            stderr,
            ..
        }) => Err(RunError::SubprocessTimedOut {
            timeout_secs,
            attempts,
            raw_text,
            stderr,
            session_slug: session_slug.to_string(),
            iterations_completed,
        }),
        Err(RunError::Interrupted { partial_result, .. }) => Err(RunError::Interrupted {
            session_slug: session_slug.to_string(),
            iterations_completed,
            partial_result,
            pending_before: Some(pending_before),
        }),
        Err(e) => Err(e),
    }
}

/// Initialize or continue a session based on configuration.
fn initialize_run_session(config: &RunConfig) -> Result<(String, PathBuf), RunError> {
    let project_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    if config.starting_iteration > 0 {
        // Continuing an existing session
        if let Some(slug) = config.slug.as_deref() {
            let dir = crate::session::session_dir(slug);
            Ok((slug.to_string(), dir))
        } else {
            // Fallback: initialize new session (shouldn't happen if starting_iteration > 0)
            Ok(initialize_session(None, &project_path, None)?)
        }
    } else {
        // First run: initialize new session with prompt
        Ok(initialize_session(
            config.slug.as_deref(),
            &project_path,
            Some(config.prompt.clone()),
        )?)
    }
}

/// State maintained across iterations.
struct IterationState {
    iterations_completed: usize,
    completion_reason: Option<CompletionReason>,
    final_pending_stories: usize,
    metrics: AccumulatedMetrics,
}

impl IterationState {
    fn new(initial_pending: usize) -> Self {
        Self {
            iterations_completed: 0,
            completion_reason: None,
            final_pending_stories: initial_pending,
            metrics: AccumulatedMetrics::default(),
        }
    }
}

/// Result of processing a single iteration.
enum SingleIterationResult {
    /// Continue to next iteration (includes any detected key action)
    Continue(Option<RunKeyAction>),
    /// Stop with completion reason
    Complete(CompletionReason),
    /// PRD unchanged - stuck state (session already finalized)
    Stuck,
}

/// Process a single iteration of the run loop.
///
/// Returns whether to continue, complete, or if stuck.
#[allow(clippy::too_many_arguments)]
fn process_single_iteration(
    config: &RunConfig,
    session_slug: &str,
    session_dir: &std::path::Path,
    iteration: usize,
    max_iterations: usize,
    relative_iteration: usize,
    state: &mut IterationState,
) -> Result<SingleIterationResult, RunError> {
    // Pre-iteration check: re-read PRD and count pending
    let prd_before = read_prd_file(&config.prd_path)?;
    let pending_before = count_pending_stories(&prd_before)?;

    // Early exit if no pending stories
    if pending_before == 0 {
        return Ok(SingleIterationResult::Complete(
            CompletionReason::AllStoriesComplete,
        ));
    }

    // Display iteration header
    let header = IterationHeader {
        iteration,
        max_iterations: Some(max_iterations),
        pending_stories: pending_before,
    };
    display_iteration_header(&header);

    // Snapshot PRD content for change detection
    let prd_snapshot = prd_before.clone();

    // Invoke LLM subprocess with retry logic
    let invocation_config = InvocationConfig {
        command: &config.command,
        max_attempts: config.max_attempts,
        timeout_secs: config.timeout_secs,
        iteration,
        theme_config: config.theme_config.as_ref(),
        session_elapsed_ms: state.metrics.elapsed_ms(),
        verbose_tools_config: &config.verbose_tools_config,
        session_slug,
        max_iterations,
    };

    let recovery_outcome = handle_subprocess_invocation(
        &invocation_config,
        session_slug,
        state.iterations_completed,
        pending_before,
    )?;

    // Extract subprocess result and key action from recovery outcome
    let result = recovery_outcome.subprocess_result;
    let key_action = recovery_outcome.key_action;

    // Write iteration log
    let mut iteration_log = build_iteration_log(iteration, pending_before, &result);
    write_iteration_log(session_dir, &iteration_log)?;

    // Check for interrupt after writing iteration log
    if signal::is_interrupted() {
        return Err(RunError::Interrupted {
            session_slug: session_slug.to_string(),
            iterations_completed: relative_iteration,
            partial_result: None,
            pending_before: None,
        });
    }

    // Capture git diff
    let diff_path = session_dir.join(format!("iteration-{}.diff", iteration));
    warn_if_err(
        capture_and_write_diff(&diff_path),
        "Failed to capture git diff",
    );

    // Display summary and accumulate metrics
    display_iteration_metrics(iteration, &result);
    state.metrics.add_from_result(&result);

    // Post-iteration: re-read PRD and check completion
    let prd_after = read_prd_file(&config.prd_path)?;
    let pending_after = count_pending_stories(&prd_after)?;
    state.final_pending_stories = pending_after;

    // Update iteration log with pending_after
    iteration_log.pending_after = pending_after;
    write_iteration_log(session_dir, &iteration_log)?;

    // Check completion conditions first (marker or all stories complete)
    if let Some(reason) = check_completion(
        pending_after,
        &result.stream_result.raw_text,
        &config.completion_marker,
    ) {
        return Ok(SingleIterationResult::Complete(reason));
    }

    // Check for stuck state (PRD unchanged)
    if !has_prd_changed(&prd_snapshot, &prd_after) {
        warn_if_err(
            finalize_session(session_slug, iteration as u32, SessionOutcome::Failed),
            "Failed to finalize session",
        );
        return Ok(SingleIterationResult::Stuck);
    }

    Ok(SingleIterationResult::Continue(key_action))
}

/// Display startup information and optional prompt.
fn display_startup(
    config: &RunConfig,
    session_slug: &str,
    session_dir: &std::path::Path,
    prd_analysis: &ralph_core::prd::PrdAnalysis,
    max_iterations: usize,
) {
    let iterations_from_arg = config.max_iterations.is_some();
    let startup_info = StartupInfo {
        slug: session_slug.to_string(),
        total_stories: prd_analysis.total_stories,
        pending_stories: prd_analysis.pending_count,
        completed_stories: prd_analysis.completed_count,
        max_iterations,
        iterations_from_arg,
        custom_prd_path: config.custom_prd_path.clone(),
        custom_command: config.custom_command,
        custom_prompt: config.custom_prompt,
        custom_completion_marker: config.custom_completion_marker,
        custom_additional_prompt: config.custom_additional_prompt,
        session_dir: session_dir.to_path_buf(),
    };
    display_startup_info(&startup_info);

    if config.show_prompt {
        let attached_files = vec![AttachedFile::new(config.prd_path.clone())];
        let prompt_display = PromptDisplay {
            prompt: &config.prompt,
            attached_files,
        };
        display_prompt(&prompt_display);
    }
}
