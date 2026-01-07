//! Iteration loop execution for the run command (Imperative Shell).
//!
//! This module orchestrates the main iteration loop that drives LLM-based
//! development. It integrates all Layer 1 features: context initialization,
//! session management, subprocess invocation, PRD parsing, completion detection,
//! and git diff capture.

#![allow(dead_code)] // Module not yet used by CLI commands

use crate::git::capture_and_write_diff;
use crate::init::{initialize_context_files, InitError};
use crate::iteration::{write_iteration_log, Chunk, IterationError, IterationLog};
use crate::session::{initialize_session, SessionError};
use crate::subprocess::{invoke_subprocess, SubprocessError};
use ralph_core::completion::{check_completion, CompletionReason};
use ralph_core::context::ContextPaths;
use ralph_core::prd::{count_pending_stories, has_prd_changed, PrdError};
use std::fs;
use std::path::PathBuf;

/// Configuration for running the iteration loop.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Maximum number of iterations to run (defaults to pending story count).
    pub max_iterations: Option<usize>,
    /// Session slug (auto-generated if None).
    pub slug: Option<String>,
    /// Command template to invoke the LLM (e.g., "claude -p {prompt}").
    pub command_template: String,
    /// Prompt text to pass to the LLM.
    pub prompt: String,
    /// Completion marker string to detect in output.
    pub completion_marker: String,
    /// Context file paths.
    pub context_paths: ContextPaths,
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

    /// Subprocess invocation failed
    #[error("Failed to invoke subprocess: {0}")]
    Subprocess(#[from] SubprocessError),

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

    // 4. Initialize session directory and metadata
    let project_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (session_slug, session_dir) = initialize_session(config.slug.as_deref(), &project_path)?;

    // 5. Execute iteration loop
    let mut iterations_completed = 0;
    let mut completion_reason = None;

    for iteration in 1..=max_iterations {
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

        // Invoke LLM subprocess
        let result = invoke_subprocess(&config.command_template)?;

        // Write iteration log
        let iteration_log = IterationLog {
            sequence: iteration as u32,
            started_at: chrono::Utc::now(),
            completed_at: chrono::Utc::now(),
            exit_code: result.exit_code,
            pending_before,
            pending_after: 0, // Will be updated below
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
            iterations_completed = iteration;
            break;
        }

        iterations_completed = iteration;
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

    fn create_test_prd(path: &PathBuf, pending: usize) {
        let mut content = String::new();
        for i in 0..pending {
            content.push_str(&format!(
                "[[stories]]\ndescription = \"Story {}\"\npasses = false\n\n",
                i + 1
            ));
        }
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn test_run_error_when_no_prd() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        let config = RunConfig {
            max_iterations: Some(1),
            slug: Some("test-slug".to_string()),
            command_template: "echo 'test'".to_string(),
            prompt: "test prompt".to_string(),
            completion_marker: "<promise>COMPLETE</promise>".to_string(),
            context_paths: paths,
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
            command_template: "echo 'test'".to_string(),
            prompt: "test prompt".to_string(),
            completion_marker: "<promise>COMPLETE</promise>".to_string(),
            context_paths: paths,
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
}
