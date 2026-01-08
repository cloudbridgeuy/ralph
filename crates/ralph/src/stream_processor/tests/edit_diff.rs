//! Edit tool diff highlighting tests for StreamProcessor.
//!
//! Tests that Edit tool results containing diffs are rendered
//! with syntax highlighting and proper formatting.

use crate::stream_processor::StreamProcessor;

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
