//! End-to-end tests for the run loop with mock Claude subprocess.
//!
//! These tests exercise the full run loop including:
//! - PRD change detection
//! - Completion marker detection
//! - Git diff capture
//! - Replay functionality
//! - Both success and failure paths
//!
//! Note: These tests modify HOME, RALPH_DATA_DIR, and current directory, which are global state.
//! We use #[serial] to ensure tests run sequentially across all test modules.

use crate::iteration::IterationLog;
use crate::paths;
use crate::replay::replay_session_with_theme;
use crate::run::{run, RunConfig};
use crate::stream_processor::VerboseToolsConfig;
use crate::summarize::SummarizeConfig;
use ralph_core::completion::CompletionReason;
use ralph_core::context::ContextPaths;
use serial_test::serial;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Helper to set up test environment variables for session storage.
/// Uses RALPH_DATA_DIR to ensure consistent paths across platforms.
fn setup_test_env_vars(temp_dir: &TempDir) {
    // Set HOME for general operations (git config, etc.)
    std::env::set_var("HOME", temp_dir.path());
    // Use RALPH_DATA_DIR to bypass platform-specific path resolution
    // This ensures sessions go to a predictable location within temp_dir
    std::env::set_var(
        paths::RALPH_DATA_DIR_ENV,
        temp_dir.path().join(".config/ralph"),
    );
}

/// Helper to clean up test environment variables.
fn cleanup_test_env_vars() {
    std::env::remove_var("HOME");
    std::env::remove_var(paths::RALPH_DATA_DIR_ENV);
}

/// Get the sessions index path for assertions.
/// Must be called while test env vars are still set.
fn get_test_sessions_index(temp_dir: &TempDir) -> std::path::PathBuf {
    temp_dir.path().join(".config/ralph/sessions.toml")
}

/// Full stream-json output that mimics Claude CLI's complete output with:
/// - System init event (session_id, model, tools)
/// - Multiple assistant messages with text and tool use
/// - User messages with tool results
/// - Result event with costs and usage
const COMPLETE_STREAM_JSON: &str = r#"{"type":"system","subtype":"init","session_id":"e2e-test-session","model":"claude-opus-4-5-20251101","tools":[{"name":"Read"},{"name":"Edit"},{"name":"Write"},{"name":"Bash"}]}
{"type":"assistant","message":{"id":"msg_01","content":[{"type":"text","text":"I'll implement the requested feature. Let me start by reading the existing code."}]}}
{"type":"assistant","message":{"id":"msg_02","content":[{"type":"tool_use","id":"toolu_read_01","name":"Read","input":{"file_path":"/src/lib.rs"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_read_01","content":"pub fn hello() {\n    println!(\"Hello\");\n}"}]}}
{"type":"assistant","message":{"id":"msg_03","content":[{"type":"text","text":"Now I'll update the code:\n\n```rust\npub fn hello() {\n    println!(\"Hello, World!\");\n}\n```\n\nThe feature has been implemented successfully."}]}}
{"type":"result","subtype":"success","total_cost_usd":0.2345,"duration_ms":12500,"num_turns":3,"usage":{"input_tokens":1500,"output_tokens":350,"cache_read_input_tokens":2000,"cache_creation_input_tokens":150}}
"#;

/// PRD with multiple pending stories for multi-iteration testing.
const MULTI_STORY_PRD: &str = r#"
[[stories]]
description = "Story 1: Implement feature A"
passes = false

[[stories]]
description = "Story 2: Add tests for feature A"
passes = false

[[stories]]
description = "Story 3: Update documentation"
passes = false
"#;

/// PRD with one story marked complete (for simulating partial progress).
const ONE_COMPLETE_PRD: &str = r#"
[[stories]]
description = "Story 1: Implement feature A"
passes = true

[[stories]]
description = "Story 2: Add tests for feature A"
passes = false

[[stories]]
description = "Story 3: Update documentation"
passes = false
"#;

/// PRD with all stories marked complete.
const ALL_COMPLETE_PRD: &str = r#"
[[stories]]
description = "Story 1: Implement feature A"
passes = true

[[stories]]
description = "Story 2: Add tests for feature A"
passes = true

[[stories]]
description = "Story 3: Update documentation"
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

/// Create a default RunConfig for e2e tests.
fn create_e2e_config(paths: ContextPaths, command: String, slug: &str) -> RunConfig {
    RunConfig {
        max_iterations: Some(5),
        slug: Some(slug.to_string()),
        command,
        completion_marker: "<promise>COMPLETE</promise>".to_string(),
        context_paths: paths,
        max_attempts: 2,
        starting_iteration: 0,
        timeout_secs: 30,
        theme_config: None,
        custom_prd_path: None,
        custom_design_path: None,
        custom_progress_path: None,
        custom_command: false,
        custom_prompt: false,
        custom_completion_marker: false,
        custom_additional_prompt: false,
        summarize_config: SummarizeConfig {
            disabled: true,
            ..Default::default()
        },
        verbose_tools_config: VerboseToolsConfig::new(),
    }
}

/// Set up test environment with PRD and design directory.
fn setup_e2e_env(temp_dir: &TempDir, prd_content: &str) -> ContextPaths {
    let paths = create_test_paths(temp_dir);

    // Create PRD directory and file
    fs::create_dir_all(paths.prd.parent().unwrap()).unwrap();
    fs::write(&paths.prd, prd_content).unwrap();

    // Create design directory
    fs::create_dir_all(paths.design.parent().unwrap()).unwrap();

    paths
}

/// Initialize a git repository in the temp directory for diff capture testing.
fn init_git_repo(temp_dir: &TempDir) {
    let temp_path = temp_dir.path();

    // git init
    Command::new("git")
        .args(["init"])
        .current_dir(temp_path)
        .output()
        .expect("Failed to run git init");

    // Configure git user for commits (required for git diff HEAD)
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_path)
        .output()
        .expect("Failed to configure git email");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_path)
        .output()
        .expect("Failed to configure git name");

    // Create initial commit so git diff HEAD works
    let placeholder = temp_path.join("README.md");
    fs::write(&placeholder, "# Test Project\n").unwrap();

    Command::new("git")
        .args(["add", "."])
        .current_dir(temp_path)
        .output()
        .expect("Failed to git add");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(temp_path)
        .output()
        .expect("Failed to git commit");
}

/// Creates a mock shell script that outputs stream-json and updates PRD.
fn create_mock_script(
    temp_dir: &TempDir,
    script_name: &str,
    stream_json: &str,
    prd_update: Option<(&str, &str)>,
    exit_code: i32,
) -> String {
    let script_path = temp_dir.path().join(script_name);

    let mut script = format!(
        r#"#!/bin/sh
# Output stream-json
cat << 'JSONEOF'
{}JSONEOF
"#,
        stream_json
    );

    // Optionally update PRD
    if let Some((prd_path, new_prd_content)) = prd_update {
        script.push_str(&format!(
            r#"# Update PRD
cat > '{}' << 'PRDEOF'
{}PRDEOF
"#,
            prd_path, new_prd_content
        ));
    }

    // Set exit code
    script.push_str(&format!("exit {}\n", exit_code));

    fs::write(&script_path, &script).unwrap();

    // Make script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    script_path.display().to_string()
}

/// Helper to get session directory path.
fn get_session_dir(slug: &str) -> std::path::PathBuf {
    crate::session::session_dir(slug)
}

/// Full end-to-end test with mock Claude subprocess.
///
/// This test exercises the full run loop:
/// - Creates a mock script mimicking Claude stream-json output
/// - Creates test PRD with pending stories
/// - Executes ralph run with mock script
/// - Verifies session created with correct slug
/// - Verifies iteration logs contain parsed metadata, tool calls, chunks
/// - Verifies git diff captured
/// - Verifies replay functionality works
#[test]
#[serial]
fn test_e2e_full_run_loop_with_mock_subprocess() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    // Set up git repo for diff capture
    init_git_repo(&temp_dir);

    // Set up test environment
    let paths = setup_e2e_env(&temp_dir, MULTI_STORY_PRD);
    let prd_path = paths.prd.display().to_string();

    // Create mock script that outputs stream-json and marks all stories complete
    // This ensures the run completes after one iteration with AllStoriesComplete
    let script_path = create_mock_script(
        &temp_dir,
        "mock_claude.sh",
        COMPLETE_STREAM_JSON,
        Some((&prd_path, ALL_COMPLETE_PRD)),
        0,
    );

    // Set environment for session storage (cross-platform)
    setup_test_env_vars(&temp_dir);
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let config = create_e2e_config(paths.clone(), script_path, "quiet-mountain");

    let result = run(config);

    // Capture session directory while env vars are still set
    let session_dir = get_session_dir("quiet-mountain");

    // Restore environment before assertions
    cleanup_test_env_vars();
    let _ = std::env::set_current_dir(&original_dir);

    // Verify run succeeded
    assert!(result.is_ok(), "Run failed: {:?}", result);
    let result = result.unwrap();

    // Verify session slug
    assert_eq!(result.slug, "quiet-mountain", "Session slug should match");

    // Verify iterations completed
    assert!(
        result.iterations_completed >= 1,
        "Should have completed at least 1 iteration"
    );

    // Verify cost tracking
    assert!(
        result.total_cost_usd.is_some(),
        "Total cost should be tracked"
    );
    assert!(
        result.total_cost_usd.unwrap() > 0.0,
        "Total cost should be positive"
    );

    // Verify token tracking
    assert!(
        result.total_input_tokens.is_some(),
        "Input tokens should be tracked"
    );
    assert!(
        result.total_output_tokens.is_some(),
        "Output tokens should be tracked"
    );

    // Verify iteration log contents (session_dir captured before cleanup)
    let log_path = session_dir.join("iteration-1.toml");

    assert!(log_path.exists(), "Iteration log should exist");

    let log_content = fs::read_to_string(&log_path).unwrap();
    let log: IterationLog = toml::from_str(&log_content).unwrap();

    // Verify metadata was extracted
    assert!(log.metadata.is_some(), "Metadata should be present");
    let metadata = log.metadata.as_ref().unwrap();
    assert_eq!(
        metadata.claude_session_id.as_deref(),
        Some("e2e-test-session"),
        "Session ID should match"
    );
    assert_eq!(
        metadata.model.as_deref(),
        Some("claude-opus-4-5-20251101"),
        "Model should match"
    );
    assert_eq!(metadata.cost_usd, Some(0.2345), "Cost should match");
    assert_eq!(metadata.duration_ms, Some(12500), "Duration should match");

    // Verify usage tracking
    assert!(metadata.usage.is_some(), "Usage should be present");
    let usage = metadata.usage.as_ref().unwrap();
    assert_eq!(usage.input_tokens, 1500, "Input tokens should match");
    assert_eq!(usage.output_tokens, 350, "Output tokens should match");

    // Verify tool calls were captured
    assert!(!log.tool_calls.is_empty(), "Tool calls should be captured");
    let read_call = log.tool_calls.iter().find(|c| c.name == "Read");
    assert!(read_call.is_some(), "Read tool call should be captured");
    let read_call = read_call.unwrap();
    assert_eq!(read_call.id, "toolu_read_01", "Tool call ID should match");
    assert!(
        read_call.result.is_some(),
        "Tool call result should be present"
    );
    assert!(
        read_call.result.as_ref().unwrap().contains("println"),
        "Tool call result should contain expected content"
    );

    // Verify chunks were captured
    assert!(!log.chunks.is_empty(), "Chunks should be captured");

    // Should have prose chunks
    let prose_chunks: Vec<_> = log
        .chunks
        .iter()
        .filter(|c| c.chunk_type == "prose")
        .collect();
    assert!(!prose_chunks.is_empty(), "Should have prose chunks");

    // Should have code chunks
    let code_chunks: Vec<_> = log
        .chunks
        .iter()
        .filter(|c| c.chunk_type == "code")
        .collect();
    assert!(!code_chunks.is_empty(), "Should have code chunks");

    // Verify at least one code chunk has rust language
    let has_rust = code_chunks
        .iter()
        .any(|c| c.language.as_deref() == Some("rust"));
    assert!(has_rust, "Should have a Rust code chunk");

    // Verify git diff was captured
    let diff_path = session_dir.join("iteration-1.diff");
    assert!(diff_path.exists(), "Git diff file should exist");
}

/// Test that PRD change detection works correctly.
///
/// The run loop should continue when PRD changes and error when it doesn't.
#[test]
#[serial]
fn test_e2e_prd_change_detection() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    init_git_repo(&temp_dir);
    let paths = setup_e2e_env(&temp_dir, MULTI_STORY_PRD);
    let prd_path = paths.prd.display().to_string();

    // Create script that marks all stories complete (simulating full completion)
    let script_path = create_mock_script(
        &temp_dir,
        "mock_claude.sh",
        COMPLETE_STREAM_JSON,
        Some((&prd_path, ALL_COMPLETE_PRD)),
        0,
    );

    setup_test_env_vars(&temp_dir);
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let config = create_e2e_config(paths, script_path, "silent-river");
    let result = run(config);

    cleanup_test_env_vars();
    let _ = std::env::set_current_dir(&original_dir);

    // Should succeed and complete with zero pending
    assert!(result.is_ok(), "Run failed: {:?}", result);
    let result = result.unwrap();

    // Should complete due to zero pending stories
    assert_eq!(
        result.completion_reason,
        Some(CompletionReason::AllStoriesComplete)
    );
    assert_eq!(result.final_pending_stories, 0);
}

/// Test that completion marker detection stops the run loop.
#[test]
#[serial]
fn test_e2e_completion_marker_detection() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    init_git_repo(&temp_dir);
    let paths = setup_e2e_env(&temp_dir, MULTI_STORY_PRD);
    let prd_path = paths.prd.display().to_string();

    // Stream JSON that includes completion marker
    let stream_with_marker = r#"{"type":"system","subtype":"init","session_id":"marker-test"}
{"type":"assistant","message":{"id":"msg_01","content":[{"type":"text","text":"All requested work is complete!\n\n<promise>COMPLETE</promise>"}]}}
{"type":"result","subtype":"success","total_cost_usd":0.05}
"#;

    let script_path = create_mock_script(
        &temp_dir,
        "mock_claude.sh",
        stream_with_marker,
        Some((&prd_path, ONE_COMPLETE_PRD)), // PRD changes but marker found
        0,
    );

    setup_test_env_vars(&temp_dir);
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let config = create_e2e_config(paths, script_path, "gentle-wave");
    let result = run(config);

    cleanup_test_env_vars();
    let _ = std::env::set_current_dir(&original_dir);

    assert!(result.is_ok(), "Run failed: {:?}", result);
    let result = result.unwrap();

    // Should complete due to marker detection
    assert_eq!(
        result.completion_reason,
        Some(CompletionReason::MarkerFound)
    );

    // Should have stopped after 1 iteration despite pending stories remaining
    assert_eq!(result.iterations_completed, 1);
}

/// Test failure path with subprocess failure and retry.
#[test]
#[serial]
fn test_e2e_subprocess_failure_with_retry() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    init_git_repo(&temp_dir);
    let paths = setup_e2e_env(&temp_dir, MULTI_STORY_PRD);

    // Create script that fails with non-zero exit code
    let failing_script = r#"#!/bin/sh
echo '{"type":"system","subtype":"init","session_id":"fail-test"}'
echo '{"type":"assistant","message":{"id":"msg_01","content":[{"type":"text","text":"Starting..."}]}}'
exit 1
"#;

    let script_path = temp_dir.path().join("failing_claude.sh");
    fs::write(&script_path, failing_script).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    setup_test_env_vars(&temp_dir);
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let mut config = create_e2e_config(paths, script_path.display().to_string(), "brave-falcon");
    config.max_attempts = 2; // Allow 2 retries (3 total attempts)

    let result = run(config);

    cleanup_test_env_vars();
    let _ = std::env::set_current_dir(&original_dir);

    // Should fail after exhausting retries
    assert!(result.is_err(), "Run should fail after exhausting retries");

    let err = result.unwrap_err();
    match err {
        crate::run::RunError::SubprocessFailed { attempts, .. } => {
            assert_eq!(attempts, 3, "Should have made 3 total attempts");
        }
        other => panic!("Expected SubprocessFailed error, got: {:?}", other),
    }
}

/// Test that replay functionality works after a successful run.
#[test]
#[serial]
fn test_e2e_replay_after_run() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    init_git_repo(&temp_dir);
    let paths = setup_e2e_env(&temp_dir, MULTI_STORY_PRD);
    let prd_path = paths.prd.display().to_string();

    let script_path = create_mock_script(
        &temp_dir,
        "mock_claude.sh",
        COMPLETE_STREAM_JSON,
        Some((&prd_path, ALL_COMPLETE_PRD)),
        0,
    );

    setup_test_env_vars(&temp_dir);
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let config = create_e2e_config(paths, script_path, "happy-sunset");
    let result = run(config);

    // Don't restore env vars yet - replay needs them

    assert!(result.is_ok(), "Run failed: {:?}", result);
    let result = result.unwrap();

    // Now test replay
    let replay_result = replay_session_with_theme(&result.slug, None, None);

    // Restore environment
    cleanup_test_env_vars();
    let _ = std::env::set_current_dir(&original_dir);

    // Verify replay succeeded
    assert!(replay_result.is_ok(), "Replay failed: {:?}", replay_result);

    let replay_result = replay_result.unwrap();
    assert_eq!(replay_result.slug, "happy-sunset");
    assert!(
        replay_result.iterations_replayed >= 1,
        "Should have replayed at least 1 iteration"
    );
}

/// Test that run loop handles multiple iterations correctly.
#[test]
#[serial]
fn test_e2e_multi_iteration_run() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    init_git_repo(&temp_dir);
    let paths = setup_e2e_env(&temp_dir, MULTI_STORY_PRD);
    let prd_path_str = paths.prd.display().to_string();

    // Create a counter file and two separate scripts
    let counter_file = temp_dir.path().join(".call_counter");
    fs::write(&counter_file, "0").unwrap();

    // Build the shell script without Rust format! to avoid brace escaping issues
    let script_content = build_progressive_script(&counter_file, &prd_path_str);
    let script_path = temp_dir.path().join("progressive_claude.sh");
    fs::write(&script_path, &script_content).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    setup_test_env_vars(&temp_dir);
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let mut config = create_e2e_config(paths, script_path.display().to_string(), "swift-eagle");
    config.max_iterations = Some(3);

    let result = run(config);

    // Capture session directory while env vars are still set
    let session_dir = get_session_dir("swift-eagle");

    cleanup_test_env_vars();
    let _ = std::env::set_current_dir(&original_dir);

    assert!(result.is_ok(), "Run failed: {:?}", result);
    let result = result.unwrap();

    // Should have completed 2 iterations (story count goes 3 -> 2 -> 0)
    assert_eq!(
        result.iterations_completed, 2,
        "Should have run 2 iterations"
    );
    assert_eq!(result.final_pending_stories, 0);

    // Verify both iteration logs exist (session_dir captured before cleanup)
    assert!(
        session_dir.join("iteration-1.toml").exists(),
        "Iteration 1 log should exist"
    );
    assert!(
        session_dir.join("iteration-2.toml").exists(),
        "Iteration 2 log should exist"
    );
}

/// Build a shell script that progressively updates PRD across multiple calls.
/// This is a helper to avoid format string escaping issues.
fn build_progressive_script(counter_file: &std::path::Path, prd_path: &str) -> String {
    let mut script = String::new();

    script.push_str("#!/bin/sh\n");
    script.push_str(&format!("COUNTER_FILE='{}'\n", counter_file.display()));
    script.push_str(&format!("PRD_PATH='{}'\n", prd_path));
    script.push('\n');
    script.push_str("# Read and increment counter\n");
    script.push_str("COUNT=$(cat \"$COUNTER_FILE\")\n");
    script.push_str("COUNT=$((COUNT + 1))\n");
    script.push_str("echo \"$COUNT\" > \"$COUNTER_FILE\"\n");
    script.push('\n');
    script.push_str("# Output stream-json\n");
    script.push_str(
        "echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"swift-eagle\"}'\n",
    );
    script.push_str("echo '{\"type\":\"assistant\",\"message\":{\"id\":\"msg_01\",\"content\":[{\"type\":\"text\",\"text\":\"Working...\"}]}}'\n");
    script
        .push_str("echo '{\"type\":\"result\",\"subtype\":\"success\",\"total_cost_usd\":0.1}'\n");
    script.push('\n');
    script.push_str("# Update PRD based on iteration\n");
    script.push_str("if [ \"$COUNT\" -eq 1 ]; then\n");
    script.push_str(&format!(
        "    cat > \"$PRD_PATH\" << 'PRDEOF'\n{}\nPRDEOF\n",
        ONE_COMPLETE_PRD
    ));
    script.push_str("elif [ \"$COUNT\" -ge 2 ]; then\n");
    script.push_str(&format!(
        "    cat > \"$PRD_PATH\" << 'PRDEOF'\n{}\nPRDEOF\n",
        ALL_COMPLETE_PRD
    ));
    script.push_str("fi\n");

    script
}

/// Test session creation with auto-generated slug.
#[test]
#[serial]
fn test_e2e_session_auto_slug_generation() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    init_git_repo(&temp_dir);
    let paths = setup_e2e_env(&temp_dir, MULTI_STORY_PRD);
    let prd_path = paths.prd.display().to_string();

    // Use ALL_COMPLETE_PRD so run completes after one iteration
    let script_path = create_mock_script(
        &temp_dir,
        "mock_claude.sh",
        COMPLETE_STREAM_JSON,
        Some((&prd_path, ALL_COMPLETE_PRD)),
        0,
    );

    setup_test_env_vars(&temp_dir);
    std::env::set_current_dir(temp_dir.path()).unwrap();

    // Don't specify slug - let it be auto-generated
    let mut config = create_e2e_config(paths, script_path, "placeholder");
    config.slug = None; // Auto-generate

    let result = run(config);

    cleanup_test_env_vars();
    let _ = std::env::set_current_dir(&original_dir);

    assert!(result.is_ok(), "Run failed: {:?}", result);
    let result = result.unwrap();

    // Verify slug is in adjective-noun format
    assert!(
        result.slug.contains('-'),
        "Auto-generated slug should be in adjective-noun format: {}",
        result.slug
    );

    // Verify slug contains only lowercase letters and single hyphen
    let parts: Vec<&str> = result.slug.split('-').collect();
    assert_eq!(
        parts.len(),
        2,
        "Slug should have exactly 2 parts: {}",
        result.slug
    );
    for part in parts {
        assert!(
            part.chars().all(|c| c.is_ascii_lowercase()),
            "Slug parts should be lowercase letters: {}",
            result.slug
        );
    }
}

/// Test that session is properly indexed in sessions.toml.
#[test]
#[serial]
fn test_e2e_session_index_population() {
    let temp_dir = TempDir::new().unwrap();
    let original_dir = std::env::current_dir().unwrap();

    init_git_repo(&temp_dir);
    let paths = setup_e2e_env(&temp_dir, MULTI_STORY_PRD);
    let prd_path = paths.prd.display().to_string();

    let script_path = create_mock_script(
        &temp_dir,
        "mock_claude.sh",
        COMPLETE_STREAM_JSON,
        Some((&prd_path, ALL_COMPLETE_PRD)),
        0,
    );

    setup_test_env_vars(&temp_dir);
    std::env::set_current_dir(temp_dir.path()).unwrap();

    let config = create_e2e_config(paths, script_path, "bright-star");
    let result = run(config);

    // Check sessions.toml path (using our predictable test path)
    let sessions_path = get_test_sessions_index(&temp_dir);

    cleanup_test_env_vars();
    let _ = std::env::set_current_dir(&original_dir);

    assert!(result.is_ok(), "Run failed: {:?}", result);
    let result = result.unwrap();

    // Verify sessions index was populated
    assert!(sessions_path.exists(), "Sessions index should exist");

    let index_content = fs::read_to_string(&sessions_path).unwrap();
    assert!(
        index_content.contains(&result.slug),
        "Sessions index should contain session slug"
    );
    assert!(
        index_content.contains("completed") || index_content.contains("Completed"),
        "Sessions index should show completion status"
    );
}
