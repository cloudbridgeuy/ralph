//! Core functionality tests for StreamProcessor.
//!
//! Tests basic processor construction, simple event handling,
//! and fundamental operations.

use crate::stream_processor::*;
use ralph_core::chunk::ChunkType;

#[test]
fn test_stream_processor_new() {
    let processor = StreamProcessor::new();
    assert!(processor.raw_text().is_empty());
    assert!(processor.parse_errors().is_empty());
}

#[test]
fn test_stream_processor_with_highlighting() {
    let processor = StreamProcessor::with_highlighting(true);
    assert!(processor.is_highlighting_enabled());

    let processor = StreamProcessor::with_highlighting(false);
    assert!(!processor.is_highlighting_enabled());
}

#[test]
fn test_process_empty_line() {
    let mut processor = StreamProcessor::new();
    let output = processor.process_line("");
    assert!(output.is_none());

    let output = processor.process_line("   ");
    assert!(output.is_none());
}

#[test]
fn test_process_malformed_json() {
    let mut processor = StreamProcessor::new();
    let output = processor.process_line("not json");
    assert!(output.is_none());
    assert_eq!(processor.parse_errors().len(), 1);
}

#[test]
fn test_process_system_event() {
    let mut processor = StreamProcessor::new();
    let line = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude"}"#;
    let _output = processor.process_line(line);
    // System events don't produce output - processor stores them for metadata extraction
}

#[test]
fn test_process_assistant_text_event() {
    let mut processor = StreamProcessor::with_highlighting(false);
    let line = r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"Hello, world!"}]}}"#;
    let _output = processor.process_line(line);

    // Text should be captured
    assert!(processor.raw_text().contains("Hello, world!"));
}

#[test]
fn test_process_result_event() {
    let mut processor = StreamProcessor::new();
    let line = r#"{"type":"result","duration_ms":1000,"total_cost_usd":0.05,"usage":{"input_tokens":100,"output_tokens":50}}"#;
    let output = processor.process_line(line);
    assert!(output.is_none()); // Result events don't produce output
}

#[test]
fn test_finish_extracts_metadata() {
    let mut processor = StreamProcessor::new();
    processor.process_line(
        r#"{"type":"system","subtype":"init","session_id":"test-session","model":"claude-3"}"#,
    );
    processor.process_line(
        r#"{"type":"result","duration_ms":5000,"total_cost_usd":0.10,"usage":{"input_tokens":200,"output_tokens":100}}"#,
    );

    let result = processor.finish();
    assert_eq!(result.metadata.session_id.as_deref(), Some("test-session"));
    assert_eq!(result.metadata.model.as_deref(), Some("claude-3"));
    assert_eq!(result.costs.cost_usd, Some(0.10));
    assert_eq!(result.costs.duration_ms, Some(5000));
}

#[test]
fn test_finish_returns_accumulated_text() {
    let mut processor = StreamProcessor::with_highlighting(false);
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"First "}]}}"#,
    );
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Second"}]}}"#,
    );

    let result = processor.finish();
    assert!(result.raw_text.contains("First"));
    assert!(result.raw_text.contains("Second"));
}

#[test]
fn test_tool_interaction_correlation() {
    let mut processor = StreamProcessor::new();

    // Tool invocation
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"/test"}}]}}"#,
    );

    // Tool result
    processor.process_line(
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":"file contents"}]}}"#,
    );

    let result = processor.finish();
    assert_eq!(result.tool_interactions.len(), 1);
    assert_eq!(result.tool_interactions[0].name, "Read");
    assert!(result.tool_interactions[0].result.is_some());
}

#[test]
fn test_code_block_detection() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // Send text with a code block
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Here is code:"}]}}"#,
    );
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"\n```rust\nfn main() {}\n```"}]}}"#,
    );

    let result = processor.finish();
    assert!(!result.chunks.is_empty());
    // Should have captured the code block
    let has_code = result
        .chunks
        .iter()
        .any(|c| matches!(c.chunk_type, ChunkType::Code { .. }));
    assert!(has_code, "Should have detected code block");
}

#[test]
fn test_diff_block_detection() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```diff\n+added\n-removed\n```"}]}}"#,
    );

    let result = processor.finish();
    let has_diff = result
        .chunks
        .iter()
        .any(|c| matches!(c.chunk_type, ChunkType::Diff));
    assert!(has_diff, "Should have detected diff block");
}

#[test]
fn test_multiple_messages() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // First message
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First message"}]}}"#,
    );

    // Second message (different ID)
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Second message"}]}}"#,
    );

    let result = processor.finish();
    assert!(result.raw_text.contains("First message"));
    assert!(result.raw_text.contains("Second message"));
}

#[test]
fn test_empty_finish() {
    let processor = StreamProcessor::new();
    let result = processor.finish();
    assert!(result.chunks.is_empty());
    assert!(result.raw_text.is_empty());
    assert!(result.tool_interactions.is_empty());
}
