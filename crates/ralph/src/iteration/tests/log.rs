//! Tests for IterationLog serialization and deserialization.

use super::super::*;
use ralph_core::stream::Usage;

// ==========================================================================
// IterationLog Tests (without metadata)
// ==========================================================================

#[test]
fn test_iteration_log_serialization() {
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
        chunks: vec![Chunk::prose("Test output".to_string())],
        output_blocks: vec![],
        strategy_name: None,
        persona: None,
    };

    let toml_str = toml::to_string_pretty(&log).unwrap();

    // Verify key fields are present
    assert!(toml_str.contains("sequence = 1"));
    assert!(toml_str.contains("exit_code = 0"));
    assert!(toml_str.contains("pending_before = 5"));
    assert!(toml_str.contains("pending_after = 4"));
    assert!(toml_str.contains("[[chunks]]"));
    assert!(toml_str.contains("type = \"prose\""));
    assert!(toml_str.contains("Test output"));
    // Metadata should not appear when None
    assert!(!toml_str.contains("[metadata]"));
    // tool_calls should not appear when empty
    assert!(!toml_str.contains("[[tool_calls]]"));
}

#[test]
fn test_iteration_log_deserialization() {
    // Note: TOML requires RFC 3339 dates without the Z suffix for bare datetime,
    // or quoted strings for full RFC 3339 with timezone
    let toml_str = r#"
        sequence = 2
        started_at = "2025-01-06T14:30:00Z"
        completed_at = "2025-01-06T14:35:00Z"
        exit_code = 0
        pending_before = 3
        pending_after = 2

        [[chunks]]
        type = "prose"
        content = "Implementation complete"
    "#;

    let log: IterationLog = toml::from_str(toml_str).unwrap();

    assert_eq!(log.sequence, 2);
    assert_eq!(log.exit_code, 0);
    assert_eq!(log.pending_before, 3);
    assert_eq!(log.pending_after, 2);
    assert!(log.metadata.is_none()); // Backward compatible - no metadata
    assert!(log.tool_calls.is_empty()); // Backward compatible - no tool_calls
    assert_eq!(log.chunks.len(), 1);
    assert_eq!(log.chunks[0].chunk_type, "prose");
    assert_eq!(log.chunks[0].content, "Implementation complete");
}

#[test]
fn test_iteration_log_empty_chunks() {
    let now = chrono::Utc::now();
    let log = IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 1,
        pending_before: 5,
        pending_after: 5,
        prompt: None,
        response: None,
        metadata: None,
        tool_calls: vec![],
        chunks: vec![],
        output_blocks: vec![],
        strategy_name: None,
        persona: None,
    };

    let toml_str = toml::to_string_pretty(&log).unwrap();
    let parsed: IterationLog = toml::from_str(&toml_str).unwrap();

    assert_eq!(parsed.chunks.len(), 0);
    assert!(parsed.tool_calls.is_empty());
}

#[test]
fn test_iteration_log_with_code_chunk() {
    let now = chrono::Utc::now();
    let log = IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 2,
        pending_after: 1,
        prompt: None,
        response: None,
        metadata: None,
        tool_calls: vec![],
        chunks: vec![
            Chunk::prose("I'll implement the function:".to_string()),
            Chunk::code(
                "fn hello() {\n    println!(\"Hello\");\n}".to_string(),
                Some("rust".to_string()),
            ),
        ],
        output_blocks: vec![],
        strategy_name: None,
        persona: None,
    };

    let toml_str = toml::to_string_pretty(&log).unwrap();

    // Verify language field is included for code chunks
    assert!(toml_str.contains("language = \"rust\""));

    // Verify deserialization preserves language
    let parsed: IterationLog = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.chunks[1].language, Some("rust".to_string()));
}

// ==========================================================================
// IterationLog Tests (with metadata)
// ==========================================================================

#[test]
fn test_iteration_log_with_metadata() {
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
            claude_session_id: Some("f5b6aaac-4316-454a-b086-a3f9e4351b1e".to_string()),
            model: Some("claude-opus-4-5-20251101".to_string()),
            cost_usd: Some(0.226354),
            duration_ms: Some(40966),
            usage: Some(Usage {
                input_tokens: 712,
                output_tokens: 2971,
                cache_read_input_tokens: Some(107476),
                cache_creation_input_tokens: Some(12504),
            }),
        }),
        tool_calls: vec![],
        chunks: vec![Chunk::prose("Test output".to_string())],
        output_blocks: vec![],
        strategy_name: None,
        persona: None,
    };

    let toml_str = toml::to_string_pretty(&log).unwrap();

    // Verify metadata section is present
    assert!(toml_str.contains("[metadata]"));
    assert!(toml_str.contains("claude_session_id = \"f5b6aaac-4316-454a-b086-a3f9e4351b1e\""));
    assert!(toml_str.contains("model = \"claude-opus-4-5-20251101\""));
    assert!(toml_str.contains("cost_usd = 0.226354"));
    assert!(toml_str.contains("duration_ms = 40966"));
    assert!(toml_str.contains("[metadata.usage]"));
    assert!(toml_str.contains("input_tokens = 712"));
}

#[test]
fn test_iteration_log_deserialization_with_metadata() {
    let toml_str = r#"
        sequence = 1
        started_at = "2025-01-06T14:30:00Z"
        completed_at = "2025-01-06T14:35:00Z"
        exit_code = 0
        pending_before = 5
        pending_after = 4

        [metadata]
        claude_session_id = "abc-123"
        model = "claude-opus-4-5"
        cost_usd = 0.05
        duration_ms = 10000

        [metadata.usage]
        input_tokens = 100
        output_tokens = 200
        cache_read_input_tokens = 50
        cache_creation_input_tokens = 25

        [[chunks]]
        type = "prose"
        content = "Implementation complete"
    "#;

    let log: IterationLog = toml::from_str(toml_str).unwrap();

    assert_eq!(log.sequence, 1);
    assert!(log.metadata.is_some());

    let metadata = log.metadata.unwrap();
    assert_eq!(metadata.claude_session_id, Some("abc-123".to_string()));
    assert_eq!(metadata.model, Some("claude-opus-4-5".to_string()));
    assert_eq!(metadata.cost_usd, Some(0.05));
    assert_eq!(metadata.duration_ms, Some(10000));

    let usage = metadata.usage.unwrap();
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 200);
    assert_eq!(usage.cache_read_input_tokens, Some(50));
    assert_eq!(usage.cache_creation_input_tokens, Some(25));
}

#[test]
fn test_iteration_log_backward_compatible_without_metadata() {
    // Old logs without metadata section should still parse
    let old_toml = r#"
        sequence = 3
        started_at = "2025-01-06T14:30:00Z"
        completed_at = "2025-01-06T14:35:00Z"
        exit_code = 0
        pending_before = 10
        pending_after = 9

        [[chunks]]
        type = "prose"
        content = "Old format log"
    "#;

    let log: IterationLog = toml::from_str(old_toml).unwrap();

    assert_eq!(log.sequence, 3);
    assert!(log.metadata.is_none());
    assert_eq!(log.chunks.len(), 1);
}

// ==========================================================================
// extract_response_text Tests
// ==========================================================================

#[test]
fn test_extract_response_text_empty() {
    use super::super::extract_response_text;
    use crate::stream_processor::OutputBlock;

    let blocks: Vec<OutputBlock> = vec![];
    let result = extract_response_text(&blocks);
    assert!(result.is_none());
}

#[test]
fn test_extract_response_text_single_text_block() {
    use super::super::extract_response_text;
    use crate::stream_processor::OutputBlock;
    use ralph_core::chunk::ParsedChunk;

    let blocks = vec![OutputBlock::text(ParsedChunk::prose("Hello, world!"))];
    let result = extract_response_text(&blocks);
    assert_eq!(result, Some("Hello, world!".to_string()));
}

#[test]
fn test_extract_response_text_multiple_text_blocks() {
    use super::super::extract_response_text;
    use crate::stream_processor::OutputBlock;
    use ralph_core::chunk::ParsedChunk;

    let blocks = vec![
        OutputBlock::text(ParsedChunk::prose("First paragraph.")),
        OutputBlock::text(ParsedChunk::code("fn main() {}", Some("rust".to_string()))),
        OutputBlock::text(ParsedChunk::prose("Second paragraph.")),
    ];
    let result = extract_response_text(&blocks);
    assert!(result.is_some());
    let text = result.unwrap();
    assert!(text.contains("First paragraph."));
    assert!(text.contains("fn main() {}"));
    assert!(text.contains("Second paragraph."));
}

#[test]
fn test_extract_response_text_ignores_non_text_blocks() {
    use super::super::extract_response_text;
    use crate::stream_processor::{OutputBlock, ToolInvocationVariant, ToolResultVariant};
    use ralph_core::chunk::ParsedChunk;

    let blocks = vec![
        OutputBlock::text(ParsedChunk::prose("Assistant response.")),
        OutputBlock::tool_invocation(
            "Bash",
            ToolInvocationVariant::Bash {
                command: "ls".to_string(),
                description: None,
            },
        ),
        OutputBlock::tool_result(
            "Bash",
            false,
            ToolResultVariant::Bash {
                content: Some("file.txt".to_string()),
                truncated: false,
            },
        ),
        OutputBlock::separator(),
    ];
    let result = extract_response_text(&blocks);
    assert_eq!(result, Some("Assistant response.".to_string()));
}

#[test]
fn test_iteration_log_with_prompt_and_response() {
    let now = chrono::Utc::now();
    let log = IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 0,
        pending_after: 0,
        prompt: Some("What is 2+2?".to_string()),
        response: Some("The answer is 4.".to_string()),
        metadata: None,
        tool_calls: vec![],
        chunks: vec![],
        output_blocks: vec![],
        strategy_name: None,
        persona: None,
    };

    let toml_str = toml::to_string_pretty(&log).unwrap();

    assert!(toml_str.contains(r#"prompt = "What is 2+2?""#));
    assert!(toml_str.contains(r#"response = "The answer is 4.""#));

    // Verify round-trip
    let parsed: IterationLog = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.prompt, Some("What is 2+2?".to_string()));
    assert_eq!(parsed.response, Some("The answer is 4.".to_string()));
}

#[test]
fn test_iteration_log_backward_compatible_without_prompt_response() {
    // Old logs without prompt/response fields should still parse
    let old_toml = r#"
        sequence = 1
        started_at = "2025-01-06T14:30:00Z"
        completed_at = "2025-01-06T14:35:00Z"
        exit_code = 0
        pending_before = 5
        pending_after = 4

        [[chunks]]
        type = "prose"
        content = "Old format without prompt/response"
    "#;

    let log: IterationLog = toml::from_str(old_toml).unwrap();

    assert!(log.prompt.is_none());
    assert!(log.response.is_none());
    assert_eq!(log.chunks.len(), 1);
}
