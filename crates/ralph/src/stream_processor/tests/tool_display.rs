//! Tool invocation display tests for StreamProcessor.
//!
//! Tests formatting and display of tool calls and their results.

use crate::stream_processor::{
    extract_key_argument, truncate_string, KeyArgument, StreamProcessor,
};

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
