//! Unit tests for run module.

use crate::run::{run, RunConfig, RunError};
use ralph_core::context::ContextPaths;
use std::fs;
use tempfile::TempDir;

fn create_test_paths(temp_dir: &TempDir) -> ContextPaths {
    ContextPaths {
        design: temp_dir.path().join(".local/designs/design.md"),
        prd: temp_dir.path().join(".local/plans/prd.toml"),
        progress: temp_dir.path().join(".local/plans/progress.txt"),
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
        timeout_secs: 600,
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
        timeout_secs: 600,
    };

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
    assert!(msg.contains("progress.txt"));
}

#[test]
fn test_no_pending_stories_error_display() {
    let err = RunError::NoPendingStories;
    let msg = format!("{}", err);
    assert!(msg.contains("No pending stories"));
}
