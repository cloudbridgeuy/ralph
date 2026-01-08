//! Enhanced Grep tool verbose mode tests for StreamProcessor.
//!
//! Tests that Grep tool invocations and results are rendered
//! with enhanced formatting when verbose mode is enabled.

use crate::highlight::ThemeConfig;
use crate::stream_processor::{StreamProcessor, VerboseToolsConfig};

#[test]
fn test_grep_verbose_invocation_shows_pattern() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"fn\\s+main","path":"/src"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show the pattern in full
    assert!(
        out.contains("fn\\s+main"),
        "Should show full pattern: {}",
        out
    );
    // Should show Pattern label
    assert!(
        out.contains("Pattern:"),
        "Should have Pattern label: {}",
        out
    );
    // Should show path
    assert!(out.contains("/src"), "Should show search path: {}", out);
}

#[test]
fn test_grep_verbose_invocation_shows_mode() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"test","output_mode":"content"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show the output mode
    assert!(out.contains("Mode:"), "Should have Mode label: {}", out);
    assert!(out.contains("content"), "Should show output mode: {}", out);
}

#[test]
fn test_grep_verbose_invocation_shows_filters() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"test","glob":"*.rs","type":"rust","-i":true}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show filters
    assert!(
        out.contains("glob: *.rs"),
        "Should show glob filter: {}",
        out
    );
    assert!(
        out.contains("type: rust"),
        "Should show type filter: {}",
        out
    );
    assert!(
        out.contains("case-insensitive"),
        "Should show case-insensitive flag: {}",
        out
    );
}

#[test]
fn test_grep_verbose_result_shows_match_count() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    // First send the invocation
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"test"}}]}}"#,
    );

    // Then send results
    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"grep_1","content":"src/main.rs\nsrc/lib.rs\nsrc/test.rs","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show match count
    assert!(
        out.contains("3 matches"),
        "Should show match count: {}",
        out
    );
    // Should show file paths
    assert!(
        out.contains("src/main.rs"),
        "Should show file paths: {}",
        out
    );
}

#[test]
fn test_grep_verbose_result_single_match_grammar() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"test"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"grep_1","content":"src/main.rs","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should use singular "match"
    assert!(
        out.contains("1 match"),
        "Should use singular 'match': {}",
        out
    );
    assert!(!out.contains("1 matches"), "Should NOT use plural: {}", out);
}

#[test]
fn test_grep_verbose_result_empty() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"nonexistent"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"grep_1","content":"","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show no matches indicator
    assert!(
        out.contains("no matches"),
        "Should show 'no matches': {}",
        out
    );
}

#[test]
fn test_grep_verbose_result_error() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"test"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"grep_1","content":"Invalid regex pattern","is_error":true}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show error indicator
    assert!(
        out.contains("Grep error") || out.contains("!"),
        "Should show error: {}",
        out
    );
}

#[test]
fn test_grep_verbose_result_truncates_large_output() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"test"}}]}}"#,
    );

    // Generate large output (>100 lines)
    let mut lines = Vec::new();
    for i in 0..150 {
        lines.push(format!("src/file_{}.rs", i));
    }
    let large_content = lines.join("\n");

    let output = processor.process_line(&format!(
        r#"{{"type":"user","message":{{"id":"user_1","content":[{{"type":"tool_result","tool_use_id":"grep_1","content":"{}","is_error":false}}]}}}}"#,
        large_content.replace('\n', "\\n")
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
fn test_grep_verbose_with_terminal_highlighting() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), true, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"fn main"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should have ANSI codes
    assert!(out.contains("\x1b["), "Should have ANSI codes: {}", out);
    // Should have cyan header
    assert!(out.contains("\x1b[36m"), "Should have cyan header: {}", out);
}

#[test]
fn test_grep_non_verbose_uses_compact_format() {
    // No verbose config - should use compact format
    let mut processor = StreamProcessor::with_options(false, true);

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"fn\\s+main\\s*\\(","path":"/src"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should NOT have the verbose format (no "Pattern:" label)
    assert!(
        !out.contains("Pattern:"),
        "Should NOT have Pattern label in non-verbose: {}",
        out
    );
    // Should still show the pattern (but in compact format)
    assert!(out.contains("fn\\s+main"), "Should show pattern: {}", out);
}

#[test]
fn test_grep_verbose_content_mode_highlights_matches() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), true, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"main","output_mode":"content"}}]}}"#,
    );

    // Content mode output format: filename:line_number:content
    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"grep_1","content":"src/main.rs:5:fn main() {","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should contain the content with highlighting
    assert!(out.contains("fn main()"), "Should show content: {}", out);
    // Should have ANSI codes for highlighting
    assert!(out.contains("\x1b["), "Should have ANSI codes: {}", out);
}

#[test]
fn test_grep_verbose_plain_text_format() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("grep"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"grep_1","name":"Grep","input":{"pattern":"test"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should NOT have ANSI codes in plain text mode
    assert!(
        !out.contains("\x1b["),
        "Should NOT have ANSI codes in plain mode: {}",
        out
    );
    // Should have plain text header
    assert!(
        out.contains("> Grep"),
        "Should have plain Grep header: {}",
        out
    );
}
