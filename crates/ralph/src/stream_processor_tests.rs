//! Tests for stream processor functionality.

use super::*;

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

// ==========================================================================
// Whitespace preservation tests
// ==========================================================================

#[test]
fn test_whitespace_blank_lines_preserved_between_paragraphs() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // Simulate: "Paragraph 1.\n\nParagraph 2."
    let output1 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Paragraph 1.\n\nParagraph 2."}]}}"#,
    );

    let result = processor.finish();

    // Should have three chunks: Paragraph 1, blank line, Paragraph 2
    assert_eq!(result.chunks.len(), 3);
    assert_eq!(result.chunks[0].content, "Paragraph 1.");
    assert_eq!(result.chunks[1].content, ""); // blank line preserved
    assert_eq!(result.chunks[2].content, "Paragraph 2.");

    // raw_text should preserve the original
    assert_eq!(result.raw_text, "Paragraph 1.\n\nParagraph 2.");

    // Output should have correct newlines
    if let Some(out) = output1 {
        // Each chunk gets a newline, so: "Paragraph 1.\n" + "\n" + "Paragraph 2.\n"
        assert_eq!(out, "Paragraph 1.\n\nParagraph 2.\n");
    }
}

#[test]
fn test_whitespace_multiple_blank_lines_preserved() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Text\n\n\nMore text"}]}}"#,
    );

    let result = processor.finish();

    // Should have: Text, blank, blank, More text
    assert_eq!(result.chunks.len(), 4);
    assert_eq!(result.chunks[0].content, "Text");
    assert_eq!(result.chunks[1].content, "");
    assert_eq!(result.chunks[2].content, "");
    assert_eq!(result.chunks[3].content, "More text");
}

#[test]
fn test_whitespace_code_block_content_preserved() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // Code with internal blank line
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```rust\nfn a() {}\n\nfn b() {}\n```"}]}}"#,
    );

    let result = processor.finish();

    // Find the code chunk
    let code_chunk = result
        .chunks
        .iter()
        .find(|c| matches!(c.chunk_type, ChunkType::Code { .. }))
        .expect("Should have code chunk");

    // Internal blank line should be preserved
    assert_eq!(code_chunk.content, "fn a() {}\n\nfn b() {}");
}

#[test]
fn test_whitespace_indentation_preserved_in_code() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```python\ndef foo():\n    x = 1\n        nested = 2\n```"}]}}"#,
    );

    let result = processor.finish();

    let code_chunk = result
        .chunks
        .iter()
        .find(|c| matches!(c.chunk_type, ChunkType::Code { .. }))
        .expect("Should have code chunk");

    // Indentation preserved exactly
    assert_eq!(
        code_chunk.content,
        "def foo():\n    x = 1\n        nested = 2"
    );
}

#[test]
fn test_whitespace_trailing_newline_in_text() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // Text with trailing newline
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Line 1\nLine 2\n"}]}}"#,
    );

    let result = processor.finish();

    // Should preserve trailing newline as empty chunk
    assert_eq!(result.chunks.len(), 3);
    assert_eq!(result.chunks[0].content, "Line 1");
    assert_eq!(result.chunks[1].content, "Line 2");
    assert_eq!(result.chunks[2].content, ""); // trailing newline
}

#[test]
fn test_whitespace_leading_spaces_preserved() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"    indented line"}]}}"#,
    );

    let result = processor.finish();

    assert_eq!(result.chunks.len(), 1);
    assert_eq!(result.chunks[0].content, "    indented line");
}

#[test]
fn test_whitespace_list_indentation_preserved() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"- Item 1\n  - Nested item\n    - Deeply nested"}]}}"#,
    );

    let result = processor.finish();

    assert_eq!(result.chunks.len(), 3);
    assert_eq!(result.chunks[0].content, "- Item 1");
    assert_eq!(result.chunks[1].content, "  - Nested item");
    assert_eq!(result.chunks[2].content, "    - Deeply nested");
}

#[test]
fn test_whitespace_blank_line_before_code_block() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Here's code:\n\n```rust\nfn main() {}\n```"}]}}"#,
    );

    let result = processor.finish();

    // Should have: prose ("Here's code:"), blank line, code block
    assert_eq!(result.chunks.len(), 3);
    assert_eq!(result.chunks[0].content, "Here's code:");
    assert_eq!(result.chunks[1].content, ""); // blank line before code
    assert!(matches!(
        result.chunks[2].chunk_type,
        ChunkType::Code { .. }
    ));
}

#[test]
fn test_whitespace_blank_line_after_code_block() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```rust\nfn main() {}\n```\n\nDone."}]}}"#,
    );

    let result = processor.finish();

    // Should have: code block, blank line, prose ("Done.")
    assert_eq!(result.chunks.len(), 3);
    assert!(matches!(
        result.chunks[0].chunk_type,
        ChunkType::Code { .. }
    ));
    assert_eq!(result.chunks[1].content, ""); // blank line after code
    assert_eq!(result.chunks[2].content, "Done.");
}

#[test]
fn test_whitespace_raw_text_matches_original() {
    let mut processor = StreamProcessor::with_highlighting(false);

    let original = "Hello\n\nWorld\n\n```rust\ncode\n```\n\nDone";
    processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"text","text":"{}"}}]}}}}"#,
        original.replace('\n', "\\n")
    ));

    let result = processor.finish();

    // raw_text should match original exactly
    assert_eq!(result.raw_text, original);
}

#[test]
fn test_whitespace_across_multiple_events() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // First event ends mid-paragraph
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Hello "}]}}"#,
    );

    // Second event continues
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"World\n\nNext paragraph"}]}}"#,
    );

    let result = processor.finish();

    // raw_text should be "Hello World\n\nNext paragraph"
    assert_eq!(result.raw_text, "Hello World\n\nNext paragraph");
}

// ==========================================================================
// Tool invocation display tests
// ==========================================================================

#[test]
fn test_tool_invocation_displayed() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Process an assistant event with a tool use
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"toolu_01","name":"Read","input":{"file_path":"/src/main.rs"}}]}}"#,
    );

    // Should return formatted tool invocation
    assert!(output.is_some());
    let out = output.unwrap();
    assert!(out.contains("Read"));
    assert!(out.contains("/src/main.rs"));
}

#[test]
fn test_tool_invocation_not_displayed_when_disabled() {
    let mut processor = StreamProcessor::with_options(false, false);

    // Process an assistant event with a tool use
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"toolu_01","name":"Read","input":{"file_path":"/src/main.rs"}}]}}"#,
    );

    // Should return None because tool display is disabled
    assert!(output.is_none());
}

#[test]
fn test_tool_result_displayed() {
    let mut processor = StreamProcessor::with_options(false, true);

    // First, process the tool call
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"toolu_01","name":"Read","input":{"file_path":"/src/main.rs"}}]}}"#,
    );

    // Then process the tool result
    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_01","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"fn main() {}","is_error":false}]}}"#,
    );

    // Should return formatted result
    assert!(output.is_some());
    let out = output.unwrap();
    assert!(out.contains("fn main()"));
}

#[test]
fn test_tool_error_displayed_distinctly() {
    let mut processor = StreamProcessor::with_options(true, true);

    // First, process the tool call
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"toolu_01","name":"Read","input":{"file_path":"/nonexistent"}}]}}"#,
    );

    // Then process an error result
    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_01","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":"File not found","is_error":true}]}}"#,
    );

    // Should return formatted error with red color
    assert!(output.is_some());
    let out = output.unwrap();
    assert!(out.contains("Error"));
    assert!(out.contains("\x1b[31m")); // Red color code
}

#[test]
fn test_multiple_concurrent_tools() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Process an assistant event with multiple tool uses
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"toolu_01","name":"Glob","input":{"pattern":"*.rs"}},{"type":"tool_use","id":"toolu_02","name":"Grep","input":{"pattern":"fn main"}}]}}"#,
    );

    // Should return both tool invocations
    assert!(output.is_some());
    let out = output.unwrap();
    assert!(out.contains("Glob"));
    assert!(out.contains("*.rs"));
    assert!(out.contains("Grep"));
    assert!(out.contains("fn main"));
}

#[test]
fn test_with_options_constructor() {
    let processor = StreamProcessor::with_options(true, false);
    assert!(processor.is_highlighting_enabled());
    assert!(!processor.is_showing_tool_invocations());

    let processor = StreamProcessor::with_options(false, true);
    assert!(!processor.is_highlighting_enabled());
    assert!(processor.is_showing_tool_invocations());
}

#[test]
fn test_tool_text_mixed_content() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Process an assistant event with both text and tool use
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Let me read the file."},{"type":"tool_use","id":"toolu_01","name":"Read","input":{"file_path":"/src/main.rs"}}]}}"#,
    );

    // Should return both text and tool invocation
    assert!(output.is_some());
    let out = output.unwrap();
    assert!(out.contains("Let me read the file"));
    assert!(out.contains("Read"));
}

#[test]
fn test_extract_key_argument_read() {
    let input = serde_json::json!({"file_path": "/src/main.rs"});
    let arg = extract_key_argument("Read", &input);
    assert_eq!(
        arg,
        Some(KeyArgument {
            value: "/src/main.rs".to_string(),
            is_path: true,
        })
    );
}

#[test]
fn test_extract_key_argument_glob() {
    let input = serde_json::json!({"pattern": "**/*.rs"});
    let arg = extract_key_argument("Glob", &input);
    assert_eq!(
        arg,
        Some(KeyArgument {
            value: "**/*.rs".to_string(),
            is_path: true,
        })
    );
}

#[test]
fn test_extract_key_argument_bash() {
    let input = serde_json::json!({"command": "cargo test"});
    let arg = extract_key_argument("Bash", &input);
    assert_eq!(
        arg,
        Some(KeyArgument {
            value: "cargo test".to_string(),
            is_path: false, // Bash commands are NOT paths, should be truncated
        })
    );
}

#[test]
fn test_extract_key_argument_unknown_tool() {
    // Unknown tool with file_path should still extract it as a path
    let input = serde_json::json!({"file_path": "/some/path"});
    let arg = extract_key_argument("UnknownTool", &input);
    assert_eq!(
        arg,
        Some(KeyArgument {
            value: "/some/path".to_string(),
            is_path: true,
        })
    );
}

#[test]
fn test_extract_key_argument_edit_is_path() {
    let input = serde_json::json!({"file_path": "/long/path/to/file.rs", "old_string": "fn foo()", "new_string": "fn bar()"});
    let arg = extract_key_argument("Edit", &input);
    assert_eq!(
        arg,
        Some(KeyArgument {
            value: "/long/path/to/file.rs".to_string(),
            is_path: true, // Edit should show full path
        })
    );
}

#[test]
fn test_extract_key_argument_write_is_path() {
    let input = serde_json::json!({"file_path": "/very/long/path/to/new/file.txt", "content": "Hello World"});
    let arg = extract_key_argument("Write", &input);
    assert_eq!(
        arg,
        Some(KeyArgument {
            value: "/very/long/path/to/new/file.txt".to_string(),
            is_path: true,
        })
    );
}

#[test]
fn test_extract_key_argument_grep_is_path() {
    // Grep pattern is shown in full (considered a path-like argument)
    let input = serde_json::json!({"pattern": "fn\\s+main\\s*\\("});
    let arg = extract_key_argument("Grep", &input);
    assert_eq!(
        arg,
        Some(KeyArgument {
            value: "fn\\s+main\\s*\\(".to_string(),
            is_path: true,
        })
    );
}

#[test]
fn test_extract_key_argument_web_fetch_is_path() {
    let input = serde_json::json!({"url": "https://example.com/very/long/path/to/resource"});
    let arg = extract_key_argument("WebFetch", &input);
    assert_eq!(
        arg,
        Some(KeyArgument {
            value: "https://example.com/very/long/path/to/resource".to_string(),
            is_path: true, // URLs shown in full
        })
    );
}

#[test]
fn test_extract_key_argument_task_is_not_path() {
    let input =
        serde_json::json!({"prompt": "Search for all files matching pattern foo and analyze them"});
    let arg = extract_key_argument("Task", &input);
    assert_eq!(
        arg,
        Some(KeyArgument {
            value: "Search for all files matching pattern foo and analyze them".to_string(),
            is_path: false, // Task prompts should be truncated
        })
    );
}

#[test]
fn test_truncate_string_short() {
    let s = "Hello";
    assert_eq!(truncate_string(s, 10), "Hello");
}

#[test]
fn test_truncate_string_long() {
    let s = "This is a very long string that should be truncated";
    let truncated = truncate_string(s, 20);
    assert_eq!(truncated, "This is a very lo...");
    assert!(truncated.ends_with("..."));
}

#[test]
fn test_truncate_string_newlines() {
    let s = "Line 1\nLine 2\nLine 3";
    let truncated = truncate_string(s, 50);
    assert!(!truncated.contains('\n'));
    assert!(truncated.contains("Line 1 Line 2 Line 3"));
}

#[test]
fn test_plain_text_tool_display() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Process an assistant event with a tool use
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"toolu_01","name":"Read","input":{"file_path":"/src/main.rs"}}]}}"#,
    );

    // Should use plain text format (no ANSI codes)
    assert!(output.is_some());
    let out = output.unwrap();
    assert!(out.starts_with(">"));
    assert!(!out.contains("\x1b[")); // No ANSI escape codes
}

// ==========================================================================
// Visual separation tests
// ==========================================================================

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

// ==========================================================================
// Full path display tests (no truncation for file paths)
// ==========================================================================

#[test]
fn test_file_path_shown_in_full_for_read() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Long file path that would be truncated at 60 chars
    let long_path =
        "/very/long/path/to/some/deeply/nested/directory/structure/containing/a/file.rs";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"Read","input":{{"file_path":"{}"}}}}]}}}}"#,
        long_path
    ));

    // Should show full path without truncation
    assert!(output.is_some());
    let out = output.unwrap();
    assert!(
        out.contains(long_path),
        "Full path should be shown without truncation: {}",
        out
    );
    assert!(!out.contains("..."), "Path should not be truncated");
}

#[test]
fn test_file_path_shown_in_full_for_edit() {
    let mut processor = StreamProcessor::with_options(false, true);

    let long_path =
        "/Users/developer/projects/my-project/src/components/deeply/nested/Component.tsx";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"Edit","input":{{"file_path":"{}","old_string":"foo","new_string":"bar"}}}}]}}}}"#,
        long_path
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    assert!(
        out.contains(long_path),
        "Full path should be shown: {}",
        out
    );
}

#[test]
fn test_file_path_shown_in_full_for_write() {
    let mut processor = StreamProcessor::with_options(false, true);

    let long_path = "/home/user/projects/workspace/src/modules/feature/implementation/handler.py";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"Write","input":{{"file_path":"{}","content":"some content"}}}}]}}}}"#,
        long_path
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    assert!(
        out.contains(long_path),
        "Full path should be shown: {}",
        out
    );
}

#[test]
fn test_glob_pattern_shown_in_full() {
    let mut processor = StreamProcessor::with_options(false, true);

    let long_pattern = "src/modules/**/components/**/*.{ts,tsx,js,jsx}";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"Glob","input":{{"pattern":"{}"}}}}]}}}}"#,
        long_pattern
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    assert!(
        out.contains(long_pattern),
        "Full pattern should be shown: {}",
        out
    );
}

#[test]
fn test_grep_pattern_shown_in_full() {
    let mut processor = StreamProcessor::with_options(false, true);

    let long_pattern = "fn\\s+\\w+\\s*\\([^)]*\\)\\s*->\\s*Result<[^>]+>";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"Grep","input":{{"pattern":"{}"}}}}]}}}}"#,
        long_pattern.replace('\\', "\\\\")
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    assert!(
        out.contains(long_pattern),
        "Full pattern should be shown: {}",
        out
    );
}

#[test]
fn test_bash_command_is_truncated() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Long bash command should be truncated
    let long_command = "git log --oneline --graph --decorate --all --color=always | head -100 && echo 'Done with git history'";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"Bash","input":{{"command":"{}"}}}}]}}}}"#,
        long_command
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Bash commands should be truncated to 60 chars
    assert!(
        out.contains("..."),
        "Bash command should be truncated: {}",
        out
    );
    assert!(
        !out.contains(long_command),
        "Full command should NOT be shown"
    );
}

#[test]
fn test_task_prompt_is_truncated() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Long task prompt should be truncated
    let long_prompt = "Search through the entire codebase for all instances of deprecated API usage and create a comprehensive report listing each occurrence with file path, line number, and suggested replacement";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"Task","input":{{"prompt":"{}"}}}}]}}}}"#,
        long_prompt
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Task prompts should be truncated
    assert!(
        out.contains("..."),
        "Task prompt should be truncated: {}",
        out
    );
    assert!(
        !out.contains(long_prompt),
        "Full prompt should NOT be shown"
    );
}

#[test]
fn test_url_shown_in_full() {
    let mut processor = StreamProcessor::with_options(false, true);

    let long_url = "https://api.example.com/v1/organizations/12345/projects/67890/resources/items?filter=active&page=1&limit=100";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"WebFetch","input":{{"url":"{}"}}}}]}}}}"#,
        long_url
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    assert!(out.contains(long_url), "Full URL should be shown: {}", out);
}

// =============================================================================
// Edit Tool Diff Highlighting Tests
// =============================================================================

#[test]
fn test_edit_tool_result_with_diff_shows_file_header() {
    let mut processor = StreamProcessor::with_options(false, true);

    // First, send the Edit tool invocation
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_edit","name":"Edit","input":{"file_path":"/src/main.rs","old_string":"fn main()","new_string":"fn main() {"}}]}}"#,
    );

    // Then send the tool result with diff-like content
    let diff_content = "-fn main()\n+fn main() {";
    let output = processor.process_line(&format!(
        r#"{{"type":"user","message":{{"id":"user_1","content":[{{"type":"tool_result","tool_use_id":"toolu_edit","content":"{}","is_error":false}}]}}}}"#,
        diff_content.replace('\n', "\\n")
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Should contain the file path header
    assert!(
        out.contains("/src/main.rs"),
        "Should show file path: {}",
        out
    );
}

#[test]
fn test_edit_tool_result_with_diff_shows_diff_fence() {
    let mut processor = StreamProcessor::with_options(false, true);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_edit","name":"Edit","input":{"file_path":"/src/lib.rs","old_string":"old","new_string":"new"}}]}}"#,
    );

    let diff_content = "-old\n+new";
    let output = processor.process_line(&format!(
        r#"{{"type":"user","message":{{"id":"user_1","content":[{{"type":"tool_result","tool_use_id":"toolu_edit","content":"{}","is_error":false}}]}}}}"#,
        diff_content.replace('\n', "\\n")
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Should wrap in diff fence
    assert!(out.contains("```diff"), "Should have diff fence: {}", out);
    assert!(out.contains("```\n"), "Should have closing fence: {}", out);
}

#[test]
fn test_edit_tool_result_with_diff_highlighting_enabled() {
    let mut processor = StreamProcessor::with_options(true, true); // highlighting enabled

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_edit","name":"Edit","input":{"file_path":"/src/test.rs","old_string":"a","new_string":"b"}}]}}"#,
    );

    let diff_content = "-a\n+b";
    let output = processor.process_line(&format!(
        r#"{{"type":"user","message":{{"id":"user_1","content":[{{"type":"tool_result","tool_use_id":"toolu_edit","content":"{}","is_error":false}}]}}}}"#,
        diff_content.replace('\n', "\\n")
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Should have ANSI color codes for the header (cyan)
    assert!(
        out.contains("\x1b[36m"),
        "Should have cyan color code: {}",
        out
    );
}

#[test]
fn test_edit_tool_result_error_not_treated_as_diff() {
    let mut processor = StreamProcessor::with_options(false, true);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_edit","name":"Edit","input":{"file_path":"/src/fail.rs","old_string":"x","new_string":"y"}}]}}"#,
    );

    // Error result should NOT be treated as a diff
    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"toolu_edit","content":"-x\n+y","is_error":true}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show error format, not diff format
    assert!(
        out.contains("Error:") || out.contains("!"),
        "Should show error format: {}",
        out
    );
    assert!(
        !out.contains("```diff"),
        "Should NOT have diff fence for errors: {}",
        out
    );
}

#[test]
fn test_edit_tool_result_without_diff_content() {
    let mut processor = StreamProcessor::with_options(false, true);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_edit","name":"Edit","input":{"file_path":"/src/simple.rs","old_string":"foo","new_string":"bar"}}]}}"#,
    );

    // Result that doesn't look like a diff
    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"toolu_edit","content":"Edit successful","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show normal result format, not diff format
    assert!(
        !out.contains("```diff"),
        "Should NOT have diff fence for non-diff content: {}",
        out
    );
}

#[test]
fn test_edit_tool_result_truncates_large_diff() {
    let mut processor = StreamProcessor::with_options(false, true);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_edit","name":"Edit","input":{"file_path":"/src/large.rs","old_string":"a","new_string":"b"}}]}}"#,
    );

    // Generate a large diff (>50 lines)
    let mut diff_lines = Vec::new();
    for i in 0..100 {
        diff_lines.push(format!("-old_line_{}", i));
        diff_lines.push(format!("+new_line_{}", i));
    }
    let large_diff = diff_lines.join("\n");

    let output = processor.process_line(&format!(
        r#"{{"type":"user","message":{{"id":"user_1","content":[{{"type":"tool_result","tool_use_id":"toolu_edit","content":"{}","is_error":false}}]}}}}"#,
        large_diff.replace('\n', "\\n")
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show truncation indicator
    assert!(
        out.contains("more lines"),
        "Should show truncation indicator: {}",
        out
    );
}

#[test]
fn test_non_edit_tool_result_not_treated_as_diff() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Read tool, not Edit
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_read","name":"Read","input":{"file_path":"/src/readme.md"}}]}}"#,
    );

    // Even if content looks like a diff, Read tool should use normal formatting
    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"toolu_read","content":"-old\n+new","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should NOT have diff formatting since it's a Read tool
    assert!(
        !out.contains("```diff"),
        "Read tool should NOT use diff format: {}",
        out
    );
}

#[test]
fn test_edit_tool_result_pending_invocation_tracking() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Send two Edit tool invocations
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"edit_1","name":"Edit","input":{"file_path":"/file1.rs","old_string":"a","new_string":"b"}},{"type":"tool_use","id":"edit_2","name":"Edit","input":{"file_path":"/file2.rs","old_string":"c","new_string":"d"}}]}}"#,
    );

    // Results come back in different order
    let output1 = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"edit_2","content":"-c\n+d","is_error":false}]}}"#,
    );
    assert!(output1.is_some());
    let out1 = output1.unwrap();
    assert!(
        out1.contains("/file2.rs"),
        "Should show correct file path for edit_2: {}",
        out1
    );

    let output2 = processor.process_line(
        r#"{"type":"user","message":{"id":"user_2","content":[{"type":"tool_result","tool_use_id":"edit_1","content":"-a\n+b","is_error":false}]}}"#,
    );
    assert!(output2.is_some());
    let out2 = output2.unwrap();
    assert!(
        out2.contains("/file1.rs"),
        "Should show correct file path for edit_1: {}",
        out2
    );
}

#[test]
fn test_edit_tool_result_with_unified_diff_format() {
    let mut processor = StreamProcessor::with_options(false, true);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_edit","name":"Edit","input":{"file_path":"/src/unified.rs","old_string":"x","new_string":"y"}}]}}"#,
    );

    // A more realistic unified diff format
    let diff_content = "@@ -1,3 +1,3 @@\n fn main() {\n-    x\n+    y\n }";
    let output = processor.process_line(&format!(
        r#"{{"type":"user","message":{{"id":"user_1","content":[{{"type":"tool_result","tool_use_id":"toolu_edit","content":"{}","is_error":false}}]}}}}"#,
        diff_content.replace('\n', "\\n")
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Should be treated as diff since it has @@ markers
    assert!(out.contains("```diff"), "Should have diff fence: {}", out);
    assert!(
        out.contains("/src/unified.rs"),
        "Should show file path: {}",
        out
    );
}
