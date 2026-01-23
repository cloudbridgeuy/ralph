//! Tests for LogToolCall type.

use super::super::*;
use crate::iteration::tool_call::truncate_result;
use ralph_core::stream::ToolInteraction;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_log_tool_call_from_interaction() {
    let interaction = ToolInteraction {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: json!({"file_path": "/src/main.rs"}),
        result: Some("fn main() {}".to_string()),
        is_error: false,
    };

    let log_call = LogToolCall::from_interaction(&interaction);

    assert_eq!(log_call.id, "toolu_01");
    assert_eq!(log_call.name, "Read");
    assert_eq!(log_call.input, json!({"file_path": "/src/main.rs"}));
    assert_eq!(log_call.result, Some("fn main() {}".to_string()));
    assert!(!log_call.result_truncated);
    assert!(!log_call.is_error);
}

#[test]
fn test_log_tool_call_from_interaction_no_result() {
    let interaction = ToolInteraction {
        id: "toolu_01".to_string(),
        name: "Edit".to_string(),
        input: json!({"file_path": "/src/main.rs", "old_string": "a", "new_string": "b"}),
        result: None,
        is_error: false,
    };

    let log_call = LogToolCall::from_interaction(&interaction);

    assert_eq!(log_call.name, "Edit");
    assert!(log_call.result.is_none());
    assert!(!log_call.result_truncated);
}

#[test]
fn test_log_tool_call_from_interaction_error() {
    let interaction = ToolInteraction {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: json!({"file_path": "/nonexistent"}),
        result: Some("File not found".to_string()),
        is_error: true,
    };

    let log_call = LogToolCall::from_interaction(&interaction);

    assert_eq!(log_call.result, Some("File not found".to_string()));
    assert!(log_call.is_error);
}

#[test]
fn test_log_tool_call_truncation() {
    // Create a large result that exceeds MAX_RESULT_SIZE
    let large_content = "x".repeat(15_000);

    let interaction = ToolInteraction {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: json!({"file_path": "/src/large_file.rs"}),
        result: Some(large_content.clone()),
        is_error: false,
    };

    let log_call = LogToolCall::from_interaction(&interaction);

    assert!(log_call.result_truncated);
    assert!(log_call.result.is_some());

    let result = log_call.result.unwrap();
    assert!(result.contains("... [truncated, 15000 bytes total]"));
    // Result should be smaller than original
    assert!(result.len() < large_content.len());
}

#[test]
fn test_log_tool_call_no_truncation_for_small_result() {
    let content = "x".repeat(5_000);

    let interaction = ToolInteraction {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: json!({"file_path": "/src/file.rs"}),
        result: Some(content.clone()),
        is_error: false,
    };

    let log_call = LogToolCall::from_interaction(&interaction);

    assert!(!log_call.result_truncated);
    assert_eq!(log_call.result, Some(content));
}

#[test]
fn test_log_tool_call_from_interactions() {
    let interactions = vec![
        ToolInteraction {
            id: "toolu_01".to_string(),
            name: "Glob".to_string(),
            input: json!({"pattern": "*.rs"}),
            result: Some("src/main.rs\nsrc/lib.rs".to_string()),
            is_error: false,
        },
        ToolInteraction {
            id: "toolu_02".to_string(),
            name: "Read".to_string(),
            input: json!({"file_path": "/src/main.rs"}),
            result: Some("fn main() {}".to_string()),
            is_error: false,
        },
    ];

    let log_calls = LogToolCall::from_interactions(&interactions);

    assert_eq!(log_calls.len(), 2);
    assert_eq!(log_calls[0].name, "Glob");
    assert_eq!(log_calls[1].name, "Read");
}

#[test]
fn test_log_tool_call_serialization() {
    let log_call = LogToolCall {
        id: "toolu_01".to_string(),
        name: "Glob".to_string(),
        input: json!({"pattern": "*.rs"}),
        result: Some("src/main.rs".to_string()),
        result_truncated: false,
        is_error: false,
    };

    let toml_str = toml::to_string(&log_call).unwrap();

    assert!(toml_str.contains("id = \"toolu_01\""));
    assert!(toml_str.contains("name = \"Glob\""));
    assert!(toml_str.contains("result = \"src/main.rs\""));
    // Boolean false fields should not appear when using skip_serializing_if
    assert!(!toml_str.contains("result_truncated"));
    assert!(!toml_str.contains("is_error"));
}

#[test]
fn test_log_tool_call_serialization_with_truncated() {
    let log_call = LogToolCall {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: json!({"file_path": "/src/main.rs"}),
        result: Some("truncated content...".to_string()),
        result_truncated: true,
        is_error: false,
    };

    let toml_str = toml::to_string(&log_call).unwrap();

    assert!(toml_str.contains("result_truncated = true"));
}

#[test]
fn test_log_tool_call_serialization_with_error() {
    let log_call = LogToolCall {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: json!({"file_path": "/nonexistent"}),
        result: Some("File not found".to_string()),
        result_truncated: false,
        is_error: true,
    };

    let toml_str = toml::to_string(&log_call).unwrap();

    assert!(toml_str.contains("is_error = true"));
}

#[test]
fn test_log_tool_call_deserialization() {
    let toml_str = r#"
        id = "toolu_01YWLzHW2VBHQSz8VV1oCGSp"
        name = "Glob"
        result = "/Users/dev/project/release.yml\n/Users/dev/project/ci.yml"

        [input]
        pattern = ".github/workflows/*.yml"
    "#;

    let log_call: LogToolCall = toml::from_str(toml_str).unwrap();

    assert_eq!(log_call.id, "toolu_01YWLzHW2VBHQSz8VV1oCGSp");
    assert_eq!(log_call.name, "Glob");
    assert!(log_call.result.is_some());
    assert!(!log_call.result_truncated);
    assert!(!log_call.is_error);
}

#[test]
fn test_iteration_log_with_tool_calls() {
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
        tool_calls: vec![
            LogToolCall {
                id: "toolu_01".to_string(),
                name: "Glob".to_string(),
                input: json!({"pattern": "*.rs"}),
                result: Some("src/main.rs".to_string()),
                result_truncated: false,
                is_error: false,
            },
            LogToolCall {
                id: "toolu_02".to_string(),
                name: "Read".to_string(),
                input: json!({"file_path": "/src/main.rs"}),
                result: Some("fn main() {}".to_string()),
                result_truncated: false,
                is_error: false,
            },
        ],
        chunks: vec![Chunk::prose("Found and read files".to_string())],
        output_blocks: vec![],
    };

    let toml_str = toml::to_string_pretty(&log).unwrap();

    assert!(toml_str.contains("[[tool_calls]]"));
    assert!(toml_str.contains("name = \"Glob\""));
    assert!(toml_str.contains("name = \"Read\""));

    // Verify round-trip
    let parsed: IterationLog = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.tool_calls.len(), 2);
    assert_eq!(parsed.tool_calls[0].name, "Glob");
    assert_eq!(parsed.tool_calls[1].name, "Read");
}

#[test]
fn test_iteration_log_deserialization_with_tool_calls() {
    let toml_str = r#"
        sequence = 1
        started_at = "2025-01-06T14:30:00Z"
        completed_at = "2025-01-06T14:35:00Z"
        exit_code = 0
        pending_before = 5
        pending_after = 4

        [[tool_calls]]
        id = "toolu_01YWLzHW2VBHQSz8VV1oCGSp"
        name = "Glob"
        result = "/Users/.../release.yml\n/Users/.../ci.yml"

        [tool_calls.input]
        pattern = ".github/workflows/*.yml"

        [[tool_calls]]
        id = "toolu_01KKvyfhUNr2Bdu32AKbDzmX"
        name = "Read"
        result = "[workspace]\nmembers = ..."
        result_truncated = true

        [tool_calls.input]
        file_path = "/Users/.../Cargo.toml"

        [[chunks]]
        type = "prose"
        content = "I'll check the workflows..."
    "#;

    let log: IterationLog = toml::from_str(toml_str).unwrap();

    assert_eq!(log.tool_calls.len(), 2);

    let first_call = &log.tool_calls[0];
    assert_eq!(first_call.name, "Glob");
    assert!(!first_call.result_truncated);

    let second_call = &log.tool_calls[1];
    assert_eq!(second_call.name, "Read");
    assert!(second_call.result_truncated);
}

#[test]
fn test_write_iteration_log_with_tool_calls() {
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
        tool_calls: vec![LogToolCall {
            id: "toolu_01".to_string(),
            name: "Glob".to_string(),
            input: json!({"pattern": "*.rs"}),
            result: Some("src/main.rs".to_string()),
            result_truncated: false,
            is_error: false,
        }],
        chunks: vec![Chunk::prose("Found files".to_string())],
        output_blocks: vec![],
    };

    let log_path = write_iteration_log(session_dir, &log).unwrap();

    let content = fs::read_to_string(&log_path).unwrap();
    let parsed: IterationLog = toml::from_str(&content).unwrap();

    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].name, "Glob");
}

#[test]
fn test_truncate_result_preserves_utf8() {
    // Test that truncation doesn't break UTF-8 characters
    // Create a string with multi-byte UTF-8 characters
    let content = "こんにちは世界".repeat(2000); // Japanese text

    let (result, truncated) = truncate_result(&content);

    assert!(truncated);
    assert!(result.is_some());

    // The result should be valid UTF-8
    let result_str = result.unwrap();
    assert!(result_str.is_char_boundary(result_str.len()));

    // Should contain the truncation message
    assert!(result_str.contains("... [truncated,"));
}
