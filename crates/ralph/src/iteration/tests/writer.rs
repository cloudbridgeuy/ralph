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
        prompt: None,
        response: None,
        metadata: None,
        tool_calls: vec![],
        chunks: vec![
            Chunk::prose("Starting work...".to_string()),
            Chunk::prose("Finished!".to_string()),
        ],
        output_blocks: vec![],
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
        prompt: None,
        response: None,
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
        output_blocks: vec![],
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
        prompt: None,
        response: None,
        metadata: None,
        tool_calls: vec![],
        chunks: vec![Chunk::prose("First iteration".to_string())],
        output_blocks: vec![],
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
        prompt: None,
        response: None,
        metadata: None,
        tool_calls: vec![],
        chunks: vec![Chunk::prose("Second iteration".to_string())],
        output_blocks: vec![],
    };
    write_iteration_log(session_dir, &log2).unwrap();

    // Verify both files exist
    assert!(session_dir.join("iteration-1.toml").exists());
    assert!(session_dir.join("iteration-2.toml").exists());
}

#[test]
fn test_count_iterations_empty_dir() {
    let temp_dir = TempDir::new().unwrap();
    let count = count_iterations(temp_dir.path()).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_count_iterations_with_iterations() {
    let temp_dir = TempDir::new().unwrap();
    let session_dir = temp_dir.path();

    let now = chrono::Utc::now();

    // Write 3 iteration logs
    for seq in 1..=3 {
        let log = IterationLog {
            sequence: seq,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 0,
            pending_after: 0,
            prompt: None,
            response: None,
            metadata: None,
            tool_calls: vec![],
            chunks: vec![],
            output_blocks: vec![],
        };
        write_iteration_log(session_dir, &log).unwrap();
    }

    let count = count_iterations(session_dir).unwrap();
    assert_eq!(count, 3);
}

#[test]
fn test_count_iterations_ignores_other_files() {
    let temp_dir = TempDir::new().unwrap();
    let session_dir = temp_dir.path();

    // Create some iteration files
    fs::write(session_dir.join("iteration-1.toml"), "").unwrap();
    fs::write(session_dir.join("iteration-2.toml"), "").unwrap();

    // Create other files that should be ignored
    fs::write(session_dir.join("session.toml"), "").unwrap();
    fs::write(session_dir.join("notes.txt"), "").unwrap();
    fs::write(session_dir.join("iteration.txt"), "").unwrap(); // Not .toml

    let count = count_iterations(session_dir).unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_load_session_iterations_empty() {
    let temp_dir = TempDir::new().unwrap();
    let logs = load_session_iterations(temp_dir.path()).unwrap();
    assert!(logs.is_empty());
}

#[test]
fn test_load_session_iterations_sorted_by_sequence() {
    let temp_dir = TempDir::new().unwrap();
    let session_dir = temp_dir.path();
    let now = chrono::Utc::now();

    // Write iterations out of order (3, 1, 2)
    for seq in [3, 1, 2] {
        let log = IterationLog {
            sequence: seq,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 0,
            pending_after: 0,
            prompt: Some(format!("Prompt {}", seq)),
            response: Some(format!("Response {}", seq)),
            metadata: None,
            tool_calls: vec![],
            chunks: vec![],
            output_blocks: vec![],
        };
        write_iteration_log(session_dir, &log).unwrap();
    }

    let logs = load_session_iterations(session_dir).unwrap();

    assert_eq!(logs.len(), 3);
    // Should be sorted by sequence (ascending)
    assert_eq!(logs[0].sequence, 1);
    assert_eq!(logs[1].sequence, 2);
    assert_eq!(logs[2].sequence, 3);
}

#[test]
fn test_load_session_iterations_ignores_non_iteration_files() {
    let temp_dir = TempDir::new().unwrap();
    let session_dir = temp_dir.path();
    let now = chrono::Utc::now();

    // Write one valid iteration
    let log = IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 0,
        pending_after: 0,
        prompt: Some("Test".to_string()),
        response: Some("Response".to_string()),
        metadata: None,
        tool_calls: vec![],
        chunks: vec![],
        output_blocks: vec![],
    };
    write_iteration_log(session_dir, &log).unwrap();

    // Write non-iteration files
    fs::write(session_dir.join("session.toml"), "[session]").unwrap();
    fs::write(session_dir.join("notes.txt"), "notes").unwrap();

    let logs = load_session_iterations(session_dir).unwrap();
    assert_eq!(logs.len(), 1);
}

#[test]
fn test_extract_conversation_messages_empty() {
    let messages = extract_conversation_messages(&[]);
    assert!(messages.is_empty());
}

#[test]
fn test_extract_conversation_messages_skips_logs_without_prompt() {
    let now = chrono::Utc::now();
    let logs = vec![
        IterationLog {
            sequence: 1,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 0,
            pending_after: 0,
            prompt: None, // No prompt - run command style
            response: Some("Response".to_string()),
            metadata: None,
            tool_calls: vec![],
            chunks: vec![],
            output_blocks: vec![],
        },
        IterationLog {
            sequence: 2,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 0,
            pending_after: 0,
            prompt: Some("Question".to_string()),
            response: Some("Answer".to_string()),
            metadata: None,
            tool_calls: vec![],
            chunks: vec![],
            output_blocks: vec![],
        },
    ];

    let messages = extract_conversation_messages(&logs);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].prompt, "Question");
    assert_eq!(messages[0].response, "Answer");
}

#[test]
fn test_extract_conversation_messages_preserves_order() {
    let now = chrono::Utc::now();
    let logs = vec![
        IterationLog {
            sequence: 1,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 0,
            pending_after: 0,
            prompt: Some("First".to_string()),
            response: Some("First response".to_string()),
            metadata: None,
            tool_calls: vec![],
            chunks: vec![],
            output_blocks: vec![],
        },
        IterationLog {
            sequence: 2,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 0,
            pending_after: 0,
            prompt: Some("Second".to_string()),
            response: Some("Second response".to_string()),
            metadata: None,
            tool_calls: vec![],
            chunks: vec![],
            output_blocks: vec![],
        },
    ];

    let messages = extract_conversation_messages(&logs);

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].prompt, "First");
    assert_eq!(messages[1].prompt, "Second");
}

#[test]
fn test_extract_conversation_messages_handles_missing_response() {
    let now = chrono::Utc::now();
    let logs = vec![IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 0,
        pending_after: 0,
        prompt: Some("Question".to_string()),
        response: None, // No response
        metadata: None,
        tool_calls: vec![],
        chunks: vec![],
        output_blocks: vec![],
    }];

    let messages = extract_conversation_messages(&logs);

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].prompt, "Question");
    assert_eq!(messages[0].response, ""); // Defaults to empty string
}

#[test]
fn test_conversation_message_new() {
    let msg = ConversationMessage::new("prompt".to_string(), "response".to_string());
    assert_eq!(msg.prompt, "prompt");
    assert_eq!(msg.response, "response");
}

#[test]
fn test_load_session_iterations_parse_error() {
    let temp_dir = TempDir::new().unwrap();
    let session_dir = temp_dir.path();

    // Write malformed TOML
    fs::write(session_dir.join("iteration-1.toml"), "not valid toml {{{{").unwrap();

    let result = load_session_iterations(session_dir);
    assert!(matches!(result, Err(IterationError::ParseLog { .. })));
}
