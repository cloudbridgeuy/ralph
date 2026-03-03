//! Visual separation tests for StreamProcessor.
//!
//! Tests that distinct assistant responses are properly separated
//! with visual indicators.
//!
//! With block buffering (prose_threshold = usize::MAX), prose is buffered and
//! only flushed when a new message arrives or at finish() time. The separator
//! logic triggers when flushed content exists or has_emitted_output is true.

use crate::stream_processor::StreamProcessor;

#[test]
fn test_visual_separation_between_responses() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // First response — prose is buffered, returns None
    let output1 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First response"}]}}"#,
    );
    assert!(output1.is_none(), "prose should be buffered");
    assert_eq!(processor.response_count(), 1);

    // Second response (different message ID) — flushes first response's prose + separator
    let output2 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Second response"}]}}"#,
    );

    // Should contain the flushed first response and separator
    assert!(output2.is_some());
    let out2 = output2.unwrap();
    assert!(
        out2.contains("First response"),
        "Should contain flushed first response: {:?}",
        out2
    );
    // Separator follows the flushed content
    assert!(
        out2.ends_with('\n'),
        "Should end with separator: {:?}",
        out2
    );
    assert_eq!(processor.response_count(), 2);

    // Second response's text comes via finish()
    let result = processor.finish();
    assert!(result.final_output.is_some());
    let final_out = result.final_output.unwrap();
    assert!(final_out.contains("Second response"));
}

#[test]
fn test_no_separator_for_first_response() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // First response — prose is buffered
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First response"}]}}"#,
    );

    // With block buffering, first prose-only response returns None
    assert!(output.is_none(), "prose should be buffered");
    assert_eq!(processor.response_count(), 1);

    // Content appears in finish()
    let result = processor.finish();
    assert!(result.final_output.is_some());
    let final_out = result.final_output.unwrap();
    assert!(
        !final_out.starts_with('\n'),
        "First response should not have leading separator"
    );
    assert!(final_out.contains("First response"));
}

#[test]
fn test_no_separator_for_same_message_id() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // First event — buffered
    let output1 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First "}]}}"#,
    );
    assert!(output1.is_none(), "prose should be buffered");

    // Second event with same message ID (continuation) — also buffered
    let output2 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"Second"}]}}"#,
    );
    // Same message ID — no flush, no separator
    assert!(output2.is_none(), "continuation should be buffered");

    // Still only one response
    assert_eq!(processor.response_count(), 1);
}

#[test]
fn test_separator_after_tool_use_cycle() {
    let mut processor = StreamProcessor::with_options(false, false); // No tool display

    // First response with text — buffered
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"Let me check"}]}}"#,
    );

    // Tool invocation (same message) — triggers flush of buffered prose
    let tool_output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"tool_use","id":"toolu_01","name":"Read","input":{}}]}}"#,
    );
    // Prose was flushed before tool invocation processing
    assert!(tool_output.is_some());
    assert!(tool_output.unwrap().contains("Let me check"));
    assert!(processor.has_emitted_output());

    // Tool result
    processor.process_line(
        r#"{"type":"user","message":{"id":"user_01","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"file content"}]}}"#,
    );

    // New assistant response (different message ID)
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Based on the file"}]}}"#,
    );

    // Should have separator (has_emitted_output is true from the flushed prose)
    assert!(output.is_some());
    let out = output.unwrap();
    assert!(
        out.starts_with('\n'),
        "Should have separator after tool cycle: {:?}",
        out
    );
    assert_eq!(processor.response_count(), 2);
}

#[test]
fn test_multiple_responses_with_separators() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // Three distinct responses
    let out1 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"One"}]}}"#,
    );
    let out2 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Two"}]}}"#,
    );
    let out3 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-3","content":[{"type":"text","text":"Three"}]}}"#,
    );

    // First response is buffered (None)
    assert!(out1.is_none());

    // Second response flushes "One" + separator
    let o2 = out2.unwrap();
    assert!(o2.contains("One"));
    assert!(o2.ends_with('\n'), "Should have separator: {:?}", o2);

    // Third response flushes "Two" + separator
    let o3 = out3.unwrap();
    assert!(o3.contains("Two"));
    assert!(o3.ends_with('\n'), "Should have separator: {:?}", o3);

    assert_eq!(processor.response_count(), 3);

    // "Three" comes via finish()
    let result = processor.finish();
    assert!(result.final_output.is_some());
    assert!(result.final_output.unwrap().contains("Three"));
}

#[test]
fn test_response_count_increments_correctly() {
    let mut processor = StreamProcessor::with_highlighting(false);

    assert_eq!(processor.response_count(), 0);
    assert!(!processor.has_emitted_output());

    // First message — prose buffered
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First"}]}}"#,
    );
    assert_eq!(processor.response_count(), 1);
    // With block buffering, has_emitted_output is false (prose still buffered)
    assert!(!processor.has_emitted_output());

    // Same message (continuation) — still buffered
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":" more"}]}}"#,
    );
    assert_eq!(processor.response_count(), 1); // Still 1

    // New message — flushes first message's prose
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Second"}]}}"#,
    );
    assert_eq!(processor.response_count(), 2);
    // Now has_emitted_output is true (flushed prose was output)
    assert!(output.is_some());
    assert!(processor.has_emitted_output());
}

#[test]
fn test_no_separator_if_no_output_yet() {
    let mut processor = StreamProcessor::with_options(false, false); // No tool display

    // First message is tool-only (no text, tools hidden)
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"tool_use","id":"toolu_01","name":"Read","input":{}}]}}"#,
    );
    // No visible output yet
    assert!(!processor.has_emitted_output());

    // Tool result (also no visible output)
    processor.process_line(
        r#"{"type":"user","message":{"id":"user_01","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"result"}]}}"#,
    );
    assert!(!processor.has_emitted_output());

    // New message with text — nothing to flush, text is buffered
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Now with text"}]}}"#,
    );

    // With block buffering, prose is buffered — no immediate output
    assert!(output.is_none(), "prose should be buffered");

    // Content appears in finish() without separator
    let result = processor.finish();
    assert!(result.final_output.is_some());
    let final_out = result.final_output.unwrap();
    assert!(
        !final_out.starts_with('\n'),
        "Should not have separator if no prior visible output: {:?}",
        final_out
    );
    assert!(final_out.contains("Now with text"));
}
