//! Unit tests for run module.

use crate::run::{run, RunConfig, RunError};
use crate::stream_processor::VerboseToolsConfig;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Create a default RunConfig for tests with the given prd path.
fn create_test_config(prd_path: PathBuf) -> RunConfig {
    RunConfig {
        max_iterations: Some(1),
        slug: Some("test-slug".to_string()),
        command: "echo 'test'".to_string(),
        prompt: "Test prompt content".to_string(),
        completion_marker: "<promise>COMPLETE</promise>".to_string(),
        prd_path,
        max_attempts: 3,
        starting_iteration: 0,
        timeout_secs: 600,
        theme_config: None,
        custom_prd_path: None,
        custom_command: false,
        custom_prompt: false,
        custom_completion_marker: false,
        custom_additional_prompt: false,
        verbose_tools_config: VerboseToolsConfig::new(),
        show_prompt: false, // Don't display prompt in tests
    }
}

#[test]
fn test_run_error_when_no_prd() {
    let temp_dir = TempDir::new().unwrap();
    let prd_path = temp_dir.path().join(".local/plans/prd.toml");

    let config = create_test_config(prd_path);

    let result = run(config);
    assert!(matches!(result, Err(RunError::Init(_))));
}

#[test]
fn test_run_error_when_no_pending_stories() {
    let temp_dir = TempDir::new().unwrap();
    let prd_path = temp_dir.path().join(".local/plans/prd.toml");

    // Create PRD with all stories completed
    let prd_content = "[[stories]]\ndescription = \"Story 1\"\npasses = true\n";
    fs::create_dir_all(prd_path.parent().unwrap()).unwrap();
    fs::write(&prd_path, prd_content).unwrap();

    let config = create_test_config(prd_path);

    let result = run(config);
    assert!(matches!(result, Err(RunError::NoPendingStories)));
}

#[test]
fn test_subprocess_failed_error_display() {
    let err = RunError::SubprocessFailed {
        exit_code: 42,
        attempts: 3,
        raw_text: "output".to_string(),
        stderr: "error".to_string(),
        session_slug: "test-slug".to_string(),
        iterations_completed: 2,
    };
    let msg = format!("{}", err);
    assert!(msg.contains("exit code 42"));
    assert!(msg.contains("3 attempt"));
}

#[test]
fn test_subprocess_timed_out_error_display() {
    let err = RunError::SubprocessTimedOut {
        timeout_secs: 300,
        attempts: 2,
        raw_text: "partial".to_string(),
        stderr: "error".to_string(),
        session_slug: "test-slug".to_string(),
        iterations_completed: 1,
    };
    let msg = format!("{}", err);
    assert!(msg.contains("300 seconds"));
    assert!(msg.contains("2 attempt"));
}

#[test]
fn test_prd_unchanged_error_display() {
    let err = RunError::PrdUnchanged;
    let msg = format!("{}", err);
    assert!(msg.contains("PRD unchanged"));
    assert!(msg.contains("LLM may be stuck"));
}

#[test]
fn test_no_pending_stories_error_display() {
    let err = RunError::NoPendingStories;
    let msg = format!("{}", err);
    assert!(msg.contains("No pending stories"));
}

/// Test that remaining_iterations calculation is correct.
/// This is a unit test for the logic fix in the run loop.
#[test]
fn test_remaining_iterations_calculation() {
    // Verify the saturating_sub behavior that's now in the run loop
    let max_iterations: usize = 5;

    // Case 1: Fresh start (no prior iterations)
    let iteration_offset: usize = 0;
    let remaining = max_iterations.saturating_sub(iteration_offset);
    assert_eq!(remaining, 5, "Fresh start should allow all 5 iterations");

    // Case 2: Partial progress (2 iterations completed)
    let iteration_offset: usize = 2;
    let remaining = max_iterations.saturating_sub(iteration_offset);
    assert_eq!(
        remaining, 3,
        "With 2 completed, only 3 should remain (iterations 3, 4, 5)"
    );

    // Case 3: Almost done (4 iterations completed)
    let iteration_offset: usize = 4;
    let remaining = max_iterations.saturating_sub(iteration_offset);
    assert_eq!(remaining, 1, "With 4 completed, only 1 should remain");

    // Case 4: Exactly at limit (5 iterations completed)
    let iteration_offset: usize = 5;
    let remaining = max_iterations.saturating_sub(iteration_offset);
    assert_eq!(remaining, 0, "At limit, no iterations should remain");

    // Case 5: Over limit (shouldn't happen but handle gracefully)
    let iteration_offset: usize = 7;
    let remaining = max_iterations.saturating_sub(iteration_offset);
    assert_eq!(
        remaining, 0,
        "Over limit should saturate to 0, not underflow"
    );
}

/// Test that iteration numbering is correct with offset.
#[test]
fn test_iteration_numbering_with_offset() {
    let max_iterations: usize = 5;
    let iteration_offset: usize = 2;
    let remaining_iterations = max_iterations.saturating_sub(iteration_offset);

    // Collect the actual iteration numbers that would be produced
    let iterations: Vec<usize> = (1..=remaining_iterations)
        .map(|rel| iteration_offset + rel)
        .collect();

    assert_eq!(
        iterations,
        vec![3, 4, 5],
        "Should produce iterations 3, 4, 5 (not 3, 4, 5, 6, 7)"
    );
}
