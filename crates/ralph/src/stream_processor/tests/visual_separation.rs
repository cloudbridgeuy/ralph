//! Visual separation tests for StreamProcessor.
//!
//! Tests that distinct assistant responses are properly separated
//! with visual indicators.

use crate::stream_processor::StreamProcessor;

#[test]
fn test_visual_separation_between_responses() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // First response
    let output1 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First response"}]}}"#,
    );
    assert!(output1.is_some());
    assert!(processor.has_emitted_output());
    assert_eq!(processor.response_count(), 1);

    // Second response (different message ID)
    let output2 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Second response"}]}}"#,
    );

    // Should have separator before second response
    assert!(output2.is_some());
    let out2 = output2.unwrap();
    assert!(
        out2.starts_with('\n'),
        "Should have separator before second response: {:?}",
        out2
    );
    assert_eq!(processor.response_count(), 2);
}

#[test]
fn test_no_separator_for_first_response() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // First response should have no leading separator
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First response"}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // First response should not start with extra separator
    assert!(
        !out.starts_with("\n\n"),
        "First response should not have leading separator"
    );
    assert_eq!(processor.response_count(), 1);
}

#[test]
fn test_no_separator_for_same_message_id() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // First event
    let output1 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First "}]}}"#,
    );
    assert!(output1.is_some());

    // Second event with same message ID (continuation)
    let output2 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"Second"}]}}"#,
    );

    // Should NOT have separator (same message)
    assert!(output2.is_some());
    let out2 = output2.unwrap();
    assert!(
        !out2.starts_with('\n'),
        "Continuation should not have separator: {:?}",
        out2
    );
    // Still only one response
    assert_eq!(processor.response_count(), 1);
}

#[test]
fn test_separator_after_tool_use_cycle() {
    let mut processor = StreamProcessor::with_options(false, false); // No tool display

    // First response with text
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"Let me check"}]}}"#,
    );

    // Tool invocation (same message)
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"tool_use","id":"toolu_01","name":"Read","input":{}}]}}"#,
    );

    // Tool result
    processor.process_line(
        r#"{"type":"user","message":{"id":"user_01","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"file content"}]}}"#,
    );

    // New assistant response (different message ID)
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Based on the file"}]}}"#,
    );

    // Should have separator
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

    // First has no separator, second and third have separators
    assert!(!out1.unwrap().starts_with('\n'));
    assert!(out2.unwrap().starts_with('\n'));
    assert!(out3.unwrap().starts_with('\n'));
    assert_eq!(processor.response_count(), 3);
}

#[test]
fn test_response_count_increments_correctly() {
    let mut processor = StreamProcessor::with_highlighting(false);

    assert_eq!(processor.response_count(), 0);
    assert!(!processor.has_emitted_output());

    // First message
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First"}]}}"#,
    );
    assert_eq!(processor.response_count(), 1);
    assert!(processor.has_emitted_output());

    // Same message (continuation)
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":" more"}]}}"#,
    );
    assert_eq!(processor.response_count(), 1); // Still 1

    // New message
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Second"}]}}"#,
    );
    assert_eq!(processor.response_count(), 2);
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

    // New message with text - should NOT have separator since nothing was shown
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Now with text"}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should NOT start with separator since there was no visible output before
    assert!(
        !out.starts_with('\n'),
        "Should not have separator if no prior visible output: {:?}",
        out
    );
}
