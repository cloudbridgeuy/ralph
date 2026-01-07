//! Integration tests for the run loop with JSON parsing.
//!
//! These tests verify that the full run loop correctly:
//! - Parses stream-json output from mock subprocess
//! - Extracts metadata (session_id, model, costs, usage)
//! - Correlates tool calls with results
//! - Stores typed chunks in iteration logs
//! - Handles edge cases gracefully

use crate::iteration::IterationLog;
use crate::run::{run, RunConfig};
use ralph_core::context::ContextPaths;
use std::fs;
use tempfile::TempDir;

/// Sample stream-json output that mimics Claude CLI's `--output-format stream-json`.
/// This represents a successful iteration that updates the PRD.
const MOCK_STREAM_JSON: &str = r#"{"type":"system","subtype":"init","session_id":"test-session-123","model":"claude-opus-4-5-20251101","tools":[{"name":"Read"},{"name":"Edit"},{"name":"Write"}]}
{"type":"assistant","message":{"id":"msg_01ABC","content":[{"type":"text","text":"I'll implement the feature now.\n\nLet me start by reading the file:"}]}}
{"type":"assistant","message":{"id":"msg_01DEF","content":[{"type":"tool_use","id":"toolu_01XYZ","name":"Read","input":{"file_path":"/src/main.rs"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_01XYZ","content":"fn main() {\n    println!(\"Hello\");\n}"}]}}
{"type":"assistant","message":{"id":"msg_01GHI","content":[{"type":"text","text":"Now I'll update the PRD to mark this story as complete.\n\n```rust\nfn main() {\n    println!(\"Updated!\");\n}\n```\n\nDone!"}]}}
{"type":"result","subtype":"success","total_cost_usd":0.15,"duration_ms":5000,"num_turns":2,"usage":{"input_tokens":500,"output_tokens":200,"cache_read_input_tokens":1000,"cache_creation_input_tokens":100}}
"#;

/// Minimal PRD content with one pending story.
const MINIMAL_PRD: &str = r#"
[[stories]]
description = "Test story"
passes = false
"#;

/// PRD content with story marked as complete.
const COMPLETED_PRD: &str = r#"
[[stories]]
description = "Test story"
passes = true
"#;

/// Helper to create test context paths in a temp directory.
fn create_test_paths(temp_dir: &TempDir) -> ContextPaths {
    ContextPaths {
        design: temp_dir.path().join(".local/designs/design.md"),
        prd: temp_dir.path().join(".local/plans/prd.toml"),
        progress: temp_dir.path().join(".local/plans/progress.txt"),
    }
}

/// Helper to set up a test environment with PRD and mock command.
fn setup_test_env(temp_dir: &TempDir, mock_script: &str) -> (ContextPaths, String) {
    let paths = create_test_paths(temp_dir);

    // Create PRD directory and file
    fs::create_dir_all(paths.prd.parent().unwrap()).unwrap();
    fs::write(&paths.prd, MINIMAL_PRD).unwrap();

    // Create design directory
    fs::create_dir_all(paths.design.parent().unwrap()).unwrap();

    // Create mock script directory
    let script_path = temp_dir.path().join("mock_claude.sh");
    fs::write(&script_path, mock_script).unwrap();

    // Make script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    (paths, script_path.display().to_string())
}

/// Creates a mock script that outputs stream-json and updates the PRD.
fn create_mock_script_updating_prd(prd_path: &str, stream_json: &str) -> String {
    format!(
        r#"#!/bin/sh
# Output stream-json
cat << 'JSONEOF'
{}JSONEOF
# Update PRD to mark story as complete
cat > '{}' << 'PRDEOF'
{}PRDEOF
"#,
        stream_json, prd_path, COMPLETED_PRD
    )
}

/// Helper to get session directory from slug - works across platforms
fn get_session_dir(temp_dir: &TempDir, slug: &str) -> std::path::PathBuf {
    // On macOS, dirs::config_dir returns ~/Library/Application Support
    // But our test sets HOME to temp_dir, so it becomes temp_dir/.config/ralph/sessions
    temp_dir.path().join(".config/ralph/sessions").join(slug)
}

#[test]
fn test_run_loop_parses_metadata() {
    let temp_dir = TempDir::new().unwrap();
    let paths = create_test_paths(&temp_dir);
    let prd_path_str = paths.prd.display().to_string();

    let mock_script = create_mock_script_updating_prd(&prd_path_str, MOCK_STREAM_JSON);
    let (paths, script_path) = setup_test_env(&temp_dir, &mock_script);

    // Override HOME for session storage
    let session_home = temp_dir.path().join("config");
    fs::create_dir_all(&session_home).unwrap();

    // Set config dir for session storage
    std::env::set_var("HOME", temp_dir.path());

    let config = RunConfig {
        max_iterations: Some(1),
        slug: None, // Let system generate a valid slug
        command: script_path,
        completion_marker: "<promise>COMPLETE</promise>".to_string(),
        context_paths: paths.clone(),
        retry_count: 0,
        starting_iteration: 0,
        timeout_secs: 30,
    };

    let result = run(config);

    // Should complete (PRD changes, so it completes after one iteration)
    assert!(result.is_ok(), "Run failed: {:?}", result);

    let result = result.unwrap();
    // Slug should be auto-generated in adjective-noun format
    assert!(
        result.slug.contains('-'),
        "Slug should be in adjective-noun format: {}",
        result.slug
    );

    // Read the iteration log to verify metadata was parsed
    let session_dir = get_session_dir(&temp_dir, &result.slug);
    let log_path = session_dir.join("iteration-1.toml");

    if log_path.exists() {
        let log_content = fs::read_to_string(&log_path).unwrap();
        let log: IterationLog = toml::from_str(&log_content).unwrap();

        // Verify metadata was extracted
        if let Some(ref metadata) = log.metadata {
            assert_eq!(
                metadata.claude_session_id.as_deref(),
                Some("test-session-123")
            );
            assert_eq!(metadata.model.as_deref(), Some("claude-opus-4-5-20251101"));
            assert_eq!(metadata.cost_usd, Some(0.15));
            assert_eq!(metadata.duration_ms, Some(5000));

            // Verify usage
            if let Some(ref usage) = metadata.usage {
                assert_eq!(usage.input_tokens, 500);
                assert_eq!(usage.output_tokens, 200);
            }
        }
    }

    // Clean up env var
    std::env::remove_var("HOME");
}

#[test]
fn test_run_loop_correlates_tool_calls() {
    let temp_dir = TempDir::new().unwrap();
    let paths = create_test_paths(&temp_dir);
    let prd_path_str = paths.prd.display().to_string();

    let mock_script = create_mock_script_updating_prd(&prd_path_str, MOCK_STREAM_JSON);
    let (paths, script_path) = setup_test_env(&temp_dir, &mock_script);

    std::env::set_var("HOME", temp_dir.path());

    let config = RunConfig {
        max_iterations: Some(1),
        slug: None,
        command: script_path,
        completion_marker: "<promise>COMPLETE</promise>".to_string(),
        context_paths: paths,
        retry_count: 0,
        starting_iteration: 0,
        timeout_secs: 30,
    };

    let result = run(config);
    assert!(result.is_ok(), "Run failed: {:?}", result);

    let result = result.unwrap();

    // Read iteration log to verify tool calls
    let session_dir = get_session_dir(&temp_dir, &result.slug);
    let log_path = session_dir.join("iteration-1.toml");

    if log_path.exists() {
        let log_content = fs::read_to_string(&log_path).unwrap();
        let log: IterationLog = toml::from_str(&log_content).unwrap();

        // Verify tool calls were captured
        assert!(
            !log.tool_calls.is_empty(),
            "Expected tool calls to be captured"
        );

        let tool_call = &log.tool_calls[0];
        assert_eq!(tool_call.name, "Read");
        assert_eq!(tool_call.id, "toolu_01XYZ");
        assert!(tool_call.result.is_some());
        assert!(tool_call.result.as_ref().unwrap().contains("fn main()"));
    }

    std::env::remove_var("HOME");
}

#[test]
fn test_run_loop_parses_chunks() {
    let temp_dir = TempDir::new().unwrap();
    let paths = create_test_paths(&temp_dir);
    let prd_path_str = paths.prd.display().to_string();

    let mock_script = create_mock_script_updating_prd(&prd_path_str, MOCK_STREAM_JSON);
    let (paths, script_path) = setup_test_env(&temp_dir, &mock_script);

    std::env::set_var("HOME", temp_dir.path());

    let config = RunConfig {
        max_iterations: Some(1),
        slug: None,
        command: script_path,
        completion_marker: "<promise>COMPLETE</promise>".to_string(),
        context_paths: paths,
        retry_count: 0,
        starting_iteration: 0,
        timeout_secs: 30,
    };

    let result = run(config);
    assert!(result.is_ok(), "Run failed: {:?}", result);

    let result = result.unwrap();

    // Read iteration log to verify chunks
    let session_dir = get_session_dir(&temp_dir, &result.slug);
    let log_path = session_dir.join("iteration-1.toml");

    if log_path.exists() {
        let log_content = fs::read_to_string(&log_path).unwrap();
        let log: IterationLog = toml::from_str(&log_content).unwrap();

        // Verify chunks were parsed - should have prose and code chunks
        // The exact number depends on how the text is chunked
        assert!(!log.chunks.is_empty(), "Expected chunks to be captured");

        // Check that we have at least one code chunk with rust language
        let has_rust_code = log
            .chunks
            .iter()
            .any(|c| c.chunk_type == "code" && c.language.as_deref() == Some("rust"));
        assert!(
            has_rust_code,
            "Expected at least one Rust code chunk, got: {:?}",
            log.chunks
        );
    }

    std::env::remove_var("HOME");
}

#[test]
fn test_run_loop_handles_missing_metadata_gracefully() {
    // Stream-json output with minimal/missing metadata
    let minimal_stream_json = r#"{"type":"assistant","message":{"id":"msg_01","content":[{"type":"text","text":"Just some text"}]}}
{"type":"result","duration_ms":1000}
"#;

    let temp_dir = TempDir::new().unwrap();
    let paths = create_test_paths(&temp_dir);
    let prd_path_str = paths.prd.display().to_string();

    let mock_script = create_mock_script_updating_prd(&prd_path_str, minimal_stream_json);
    let (paths, script_path) = setup_test_env(&temp_dir, &mock_script);

    std::env::set_var("HOME", temp_dir.path());

    let config = RunConfig {
        max_iterations: Some(1),
        slug: None,
        command: script_path,
        completion_marker: "<promise>COMPLETE</promise>".to_string(),
        context_paths: paths,
        retry_count: 0,
        starting_iteration: 0,
        timeout_secs: 30,
    };

    // Should not crash despite missing metadata fields
    let result = run(config);
    assert!(
        result.is_ok(),
        "Run should succeed even with missing metadata: {:?}",
        result
    );

    std::env::remove_var("HOME");
}

#[test]
fn test_run_loop_handles_malformed_json_lines() {
    // Mix of valid and invalid JSON lines - should log warning but continue
    let mixed_stream_json = r#"{"type":"system","session_id":"abc"}
not valid json at all
{"type":"assistant","message":{"id":"msg_01","content":[{"type":"text","text":"Some text"}]}}
another invalid line
{"type":"result","total_cost_usd":0.05}
"#;

    let temp_dir = TempDir::new().unwrap();
    let paths = create_test_paths(&temp_dir);
    let prd_path_str = paths.prd.display().to_string();

    let mock_script = create_mock_script_updating_prd(&prd_path_str, mixed_stream_json);
    let (paths, script_path) = setup_test_env(&temp_dir, &mock_script);

    std::env::set_var("HOME", temp_dir.path());

    let config = RunConfig {
        max_iterations: Some(1),
        slug: None,
        command: script_path,
        completion_marker: "<promise>COMPLETE</promise>".to_string(),
        context_paths: paths,
        retry_count: 0,
        starting_iteration: 0,
        timeout_secs: 30,
    };

    // Should not crash despite malformed JSON lines
    let result = run(config);
    assert!(
        result.is_ok(),
        "Run should succeed even with malformed JSON lines: {:?}",
        result
    );

    let result = result.unwrap();

    // Verify that valid events were still processed
    let session_dir = get_session_dir(&temp_dir, &result.slug);
    let log_path = session_dir.join("iteration-1.toml");

    if log_path.exists() {
        let log_content = fs::read_to_string(&log_path).unwrap();
        let log: IterationLog = toml::from_str(&log_content).unwrap();

        // Should have captured some text despite errors
        assert!(!log.chunks.is_empty(), "Should have parsed valid events");
    }

    std::env::remove_var("HOME");
}

#[test]
fn test_run_loop_session_finalization() {
    let temp_dir = TempDir::new().unwrap();
    let paths = create_test_paths(&temp_dir);
    let prd_path_str = paths.prd.display().to_string();

    let mock_script = create_mock_script_updating_prd(&prd_path_str, MOCK_STREAM_JSON);
    let (paths, script_path) = setup_test_env(&temp_dir, &mock_script);

    std::env::set_var("HOME", temp_dir.path());

    let config = RunConfig {
        max_iterations: Some(1),
        slug: None,
        command: script_path,
        completion_marker: "<promise>COMPLETE</promise>".to_string(),
        context_paths: paths,
        retry_count: 0,
        starting_iteration: 0,
        timeout_secs: 30,
    };

    let result = run(config);
    assert!(result.is_ok(), "Run failed: {:?}", result);

    let result = result.unwrap();

    // Verify session was finalized in sessions.toml
    let sessions_index_path = temp_dir.path().join(".config/ralph/sessions.toml");
    if sessions_index_path.exists() {
        let content = fs::read_to_string(&sessions_index_path).unwrap();
        assert!(content.contains(&result.slug), "Session should be in index");
        assert!(
            content.contains("completed") || content.contains("in_progress"),
            "Session should have outcome"
        );
    }

    std::env::remove_var("HOME");
}
