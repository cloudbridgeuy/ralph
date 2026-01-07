//! Iteration loop execution for the run command (Imperative Shell).
//!
//! This module orchestrates the main iteration loop that drives LLM-based
//! development. It integrates all Layer 1 features: context initialization,
//! session management, subprocess invocation, PRD parsing, completion detection,
//! and git diff capture.

use crate::git::capture_and_write_diff;
use crate::init::{initialize_context_files, InitError};
use crate::iteration::{write_iteration_log, Chunk, IterationError, IterationLog};
use crate::session::{finalize_session, initialize_session, SessionError};
use crate::subprocess::{invoke_subprocess, SubprocessError};
use ralph_core::completion::{check_completion, CompletionReason};
use ralph_core::context::ContextPaths;
use ralph_core::prd::{count_pending_stories, has_prd_changed, PrdError};
use ralph_core::session::SessionOutcome;
use std::fs;
use std::path::PathBuf;

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
        stdout: String,
        stderr: String,
        /// Session slug for later finalization
        session_slug: String,
        /// Number of iterations completed before failure
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

    // 2. Read PRD and count pending stories
    let prd_content = read_prd_file(&config.context_paths.prd)?;
    let pending_count = count_pending_stories(&prd_content)?;

    // Pre-check: exit if zero pending stories
    if pending_count == 0 {
        return Err(RunError::NoPendingStories);
    }

    // 3. Determine max iterations (use provided or default to pending count)
    let max_iterations = config.max_iterations.unwrap_or(pending_count);

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

    // 5. Execute iteration loop
    let mut iterations_completed = 0;
    let mut completion_reason = None;

    // Calculate iteration numbers: if continuing a session, offset by starting_iteration
    let iteration_offset = config.starting_iteration;

    for relative_iteration in 1..=max_iterations {
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

        // Snapshot PRD content for change detection
        let prd_snapshot = prd_before.clone();

        // Invoke LLM subprocess with retry logic
        let result = match invoke_with_retries(&config.command, config.retry_count, iteration) {
            Ok(r) => r,
            Err(RunError::SubprocessFailed {
                exit_code,
                attempts,
                stdout,
                stderr,
                ..
            }) => {
                // Subprocess failed - return error with session info for caller to finalize
                // The caller will determine whether to mark as "failed" or "aborted"
                return Err(RunError::SubprocessFailed {
                    exit_code,
                    attempts,
                    stdout,
                    stderr,
                    session_slug,
                    iterations_completed,
                });
            }
            Err(e) => return Err(e),
        };

        // Write iteration log
        // Note: metadata will be populated from JSON streaming output in a future story
        let iteration_log = IterationLog {
            sequence: iteration as u32,
            started_at: chrono::Utc::now(),
            completed_at: chrono::Utc::now(),
            exit_code: result.exit_code,
            pending_before,
            pending_after: 0,   // Will be updated below
            metadata: None,     // TODO: Extract from JSON streaming output
            tool_calls: vec![], // TODO: Extract from JSON streaming output
            chunks: vec![Chunk::prose(result.stdout.clone())],
        };

        write_iteration_log(&session_dir, &iteration_log)?;

        // Capture git diff
        let diff_path = session_dir.join(format!("iteration-{}.diff", iteration));
        if let Err(e) = capture_and_write_diff(&diff_path) {
            eprintln!("Warning: Failed to capture git diff: {}", e);
        }

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

        // Check completion conditions
        if let Some(reason) =
            check_completion(pending_after, &result.stdout, &config.completion_marker)
        {
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

/// Invoke subprocess with automatic retries on failure.
///
/// This function wraps `invoke_subprocess` with retry logic:
/// - On non-zero exit code, prints stdout/stderr and retries
/// - Prints attempt number for each retry
/// - Returns error with captured output if all retries exhausted
///
/// # Arguments
///
/// * `command` - The command to execute
/// * `retry_count` - Number of retries (0 means run once with no retries)
/// * `iteration` - Current iteration number (for logging context)
///
/// # Returns
///
/// Returns the `SubprocessResult` on success (exit code 0).
fn invoke_with_retries(
    command: &str,
    retry_count: usize,
    iteration: usize,
) -> Result<crate::subprocess::SubprocessResult, RunError> {
    let max_attempts = retry_count + 1; // retry_count of 3 means 4 total attempts

    for attempt in 1..=max_attempts {
        let result = invoke_subprocess(command)?;

        if result.exit_code == 0 {
            return Ok(result);
        }

        // Non-zero exit code - handle retry
        eprintln!(
            "\n[Iteration {}] LLM subprocess failed with exit code {} (attempt {}/{})",
            iteration, result.exit_code, attempt, max_attempts
        );

        // Print captured output from failed attempt
        if !result.stdout.is_empty() {
            eprintln!("\n--- stdout ---");
            eprint!("{}", result.stdout);
            if !result.stdout.ends_with('\n') {
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
                stdout: result.stdout,
                stderr: result.stderr,
                session_slug: String::new(),
                iterations_completed: 0,
            });
        }
    }

    // This should never be reached due to the return in the loop
    unreachable!("Loop should have returned or errored")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_paths(temp_dir: &TempDir) -> ContextPaths {
        ContextPaths {
            design: temp_dir.path().join(".claude/designs/design.md"),
            prd: temp_dir.path().join(".claude/plans/prd.toml"),
            progress: temp_dir.path().join(".claude/plans/progress.txt"),
        }
    }

    #[test]
    fn test_run_error_when_no_prd() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        let config = RunConfig {
            max_iterations: Some(1),
            slug: Some("test-slug".to_string()),
            command: "echo 'test'".to_string(),
            completion_marker: "<promise>COMPLETE</promise>".to_string(),
            context_paths: paths,
            retry_count: 3,
            starting_iteration: 0,
        };

        let result = run(config);
        assert!(matches!(result, Err(RunError::Init(_))));
    }

    #[test]
    fn test_run_error_when_no_pending_stories() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        // Create PRD with all stories completed
        let prd_content = "[[stories]]\ndescription = \"Story 1\"\npasses = true\n";
        fs::create_dir_all(paths.prd.parent().unwrap()).unwrap();
        fs::write(&paths.prd, prd_content).unwrap();

        let config = RunConfig {
            max_iterations: Some(1),
            slug: Some("test-slug".to_string()),
            command: "echo 'test'".to_string(),
            completion_marker: "<promise>COMPLETE</promise>".to_string(),
            context_paths: paths,
            retry_count: 3,
            starting_iteration: 0,
        };

        let result = run(config);
        assert!(matches!(result, Err(RunError::NoPendingStories)));
    }

    #[test]
    fn test_read_prd_file_success() {
        let temp_dir = TempDir::new().unwrap();
        let prd_path = temp_dir.path().join("prd.toml");
        fs::write(&prd_path, "test content").unwrap();

        let content = read_prd_file(&prd_path).unwrap();
        assert_eq!(content, "test content");
    }

    #[test]
    fn test_read_prd_file_missing() {
        let temp_dir = TempDir::new().unwrap();
        let prd_path = temp_dir.path().join("missing.toml");

        let result = read_prd_file(&prd_path);
        assert!(matches!(result, Err(RunError::ReadPrd { .. })));
    }

    #[test]
    fn test_invoke_with_retries_success_first_attempt() {
        let result = invoke_with_retries("echo 'success'", 3, 1).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("success"));
    }

    #[test]
    fn test_invoke_with_retries_fails_all_attempts() {
        // exit 1 always fails, so all retries should be exhausted
        let result = invoke_with_retries("exit 42", 2, 1);
        assert!(matches!(
            result,
            Err(RunError::SubprocessFailed {
                exit_code: 42,
                attempts: 3, // 2 retries + 1 initial = 3 attempts
                ..
            })
        ));
    }

    #[test]
    fn test_invoke_with_retries_zero_retries() {
        // Zero retries means run once
        let result = invoke_with_retries("exit 1", 0, 1);
        assert!(matches!(
            result,
            Err(RunError::SubprocessFailed {
                exit_code: 1,
                attempts: 1,
                ..
            })
        ));
    }

    #[test]
    fn test_invoke_with_retries_captures_output() {
        // Verify stdout and stderr are captured in the error
        let result = invoke_with_retries("echo 'out'; echo 'err' >&2; exit 5", 0, 1);
        match result {
            Err(RunError::SubprocessFailed {
                stdout,
                stderr,
                exit_code,
                ..
            }) => {
                assert_eq!(exit_code, 5);
                assert!(stdout.contains("out"));
                assert!(stderr.contains("err"));
            }
            _ => panic!("Expected SubprocessFailed error"),
        }
    }

    #[test]
    fn test_subprocess_failed_error_display() {
        let err = RunError::SubprocessFailed {
            exit_code: 42,
            attempts: 3,
            stdout: "output".to_string(),
            stderr: "error".to_string(),
            session_slug: "test-slug".to_string(),
            iterations_completed: 2,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("exit code 42"));
        assert!(msg.contains("3 attempt"));
    }
}
