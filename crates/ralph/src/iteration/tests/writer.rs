//! Tests for file writing operations.

use super::super::*;
use ralph_core::stream::Usage;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_write_iteration_log() {
    let temp_dir = TempDir::new().unwrap();
    let session_dir = temp_dir.path();

    let now = chrono::Utc::now();
    let log = IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 5,
        pending_after: 4,
        metadata: None,
        tool_calls: vec![],
        chunks: vec![
            Chunk::prose("Starting work...".to_string()),
            Chunk::prose("Finished!".to_string()),
        ],
    };

    let log_path = write_iteration_log(session_dir, &log).unwrap();

    // Verify file was created
    assert!(log_path.exists());
    assert!(log_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with("iteration-1.toml"));

    // Verify content can be read back
    let content = fs::read_to_string(&log_path).unwrap();
    let parsed: IterationLog = toml::from_str(&content).unwrap();

    assert_eq!(parsed.sequence, 1);
    assert_eq!(parsed.exit_code, 0);
    assert_eq!(parsed.chunks.len(), 2);
}

#[test]
fn test_write_iteration_log_with_metadata() {
    let temp_dir = TempDir::new().unwrap();
    let session_dir = temp_dir.path();

    let now = chrono::Utc::now();
    let log = IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 5,
        pending_after: 4,
        metadata: Some(LogMetadata {
            claude_session_id: Some("test-session-id".to_string()),
            model: Some("claude-opus-4-5".to_string()),
            cost_usd: Some(0.123),
            duration_ms: Some(5000),
            usage: Some(Usage {
                input_tokens: 500,
                output_tokens: 1000,
                cache_read_input_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
            }),
        }),
        tool_calls: vec![],
        chunks: vec![Chunk::prose("Test output".to_string())],
    };

    let log_path = write_iteration_log(session_dir, &log).unwrap();

    // Verify content can be read back with metadata
    let content = fs::read_to_string(&log_path).unwrap();
    let parsed: IterationLog = toml::from_str(&content).unwrap();

    assert!(parsed.metadata.is_some());
    let metadata = parsed.metadata.unwrap();
    assert_eq!(
        metadata.claude_session_id,
        Some("test-session-id".to_string())
    );
    assert_eq!(metadata.cost_usd, Some(0.123));
}

#[test]
fn test_write_multiple_iteration_logs() {
    let temp_dir = TempDir::new().unwrap();
    let session_dir = temp_dir.path();

    let now = chrono::Utc::now();

    // Write iteration 1
    let log1 = IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 5,
        pending_after: 4,
        metadata: None,
        tool_calls: vec![],
        chunks: vec![Chunk::prose("First iteration".to_string())],
    };
    write_iteration_log(session_dir, &log1).unwrap();

    // Write iteration 2
    let log2 = IterationLog {
        sequence: 2,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 4,
        pending_after: 3,
        metadata: None,
        tool_calls: vec![],
        chunks: vec![Chunk::prose("Second iteration".to_string())],
    };
    write_iteration_log(session_dir, &log2).unwrap();

    // Verify both files exist
    assert!(session_dir.join("iteration-1.toml").exists());
    assert!(session_dir.join("iteration-2.toml").exists());
}
