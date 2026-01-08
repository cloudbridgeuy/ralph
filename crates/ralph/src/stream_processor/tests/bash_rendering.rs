//! Enhanced Bash tool rendering tests for StreamProcessor.
//!
//! Tests that Bash tool invocations and results are rendered
//! with proper formatting and syntax highlighting.

use crate::stream_processor::StreamProcessor;

#[test]
fn test_bash_command_shown_in_full_without_truncation() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Long bash command should be shown in full (not truncated)
    let long_command =
        "git log --oneline --graph --decorate --all --color=always | head -100 && echo 'Done'";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"Bash","input":{{"command":"{}"}}}}]}}}}"#,
        long_command
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Should contain the full command
    assert!(
        out.contains(long_command),
        "Full command should be shown: {}",
        out
    );
    // Should NOT be truncated (no ...)
    assert!(
        !out.contains("..."),
        "Command should NOT be truncated: {}",
        out
    );
}

#[test]
fn test_bash_command_with_terminal_highlighting() {
    let mut processor = StreamProcessor::with_options(true, true);

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"ls -la /tmp"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should have ANSI codes for highlighting
    assert!(out.contains("\x1b["), "Should have ANSI codes: {}", out);
    // Should have the Bash header
    assert!(out.contains("Bash"), "Should have Bash header: {}", out);
}

#[test]
fn test_bash_command_plain_text_format() {
    let mut processor = StreamProcessor::with_options(false, true);

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"echo hello"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should NOT have ANSI codes
    assert!(
        !out.contains("\x1b["),
        "Should NOT have ANSI codes: {}",
        out
    );
    // Should have plain text format
    assert!(
        out.contains("> Bash"),
        "Should have plain Bash header: {}",
        out
    );
    assert!(
        out.contains("echo hello"),
        "Should contain command: {}",
        out
    );
}

#[test]
fn test_bash_multiline_command_in_code_block() {
    let mut processor = StreamProcessor::with_options(false, true);

    // Multi-line command
    let command = "for f in *.txt; do\n  echo \"$f\"\n  cat \"$f\"\ndone";

    let output = processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"tool_use","id":"toolu_01","name":"Bash","input":{{"command":"{}"}}}}]}}}}"#,
        command.replace('\n', "\\n").replace('"', "\\\"")
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Multi-line should be wrapped in ```sh block
    assert!(
        out.contains("```sh"),
        "Should have sh code block fence: {}",
        out
    );
    assert!(
        out.contains("for f in *.txt"),
        "Should contain loop start: {}",
        out
    );
}

#[test]
fn test_bash_single_line_inline_format() {
    let mut processor = StreamProcessor::with_options(false, true);

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"pwd"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Single-line should NOT be in code block
    assert!(
        !out.contains("```sh"),
        "Single-line should NOT have code fence: {}",
        out
    );
    // Should have indented inline format
    assert!(
        out.contains("  pwd"),
        "Should have indented command: {}",
        out
    );
}

#[test]
fn test_bash_tool_result_success_with_output() {
    let mut processor = StreamProcessor::with_options(false, true);

    // First send the Bash invocation
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"bash_1","name":"Bash","input":{"command":"ls -la"}}]}}"#,
    );

    // Then send the result
    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"bash_1","content":"total 4\ndrwxr-xr-x  2 user user 4096 Jan  1 12:00 .\n-rw-r--r--  1 user user  123 Jan  1 12:00 file.txt","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should contain the output
    assert!(out.contains("file.txt"), "Should contain output: {}", out);
    // Should NOT have error indicator
    assert!(
        !out.contains("Exit code: non-zero"),
        "Should NOT have error: {}",
        out
    );
}

#[test]
fn test_bash_tool_result_error_shows_exit_code() {
    let mut processor = StreamProcessor::with_options(true, true);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"bash_1","name":"Bash","input":{"command":"cat nonexistent"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"bash_1","content":"cat: nonexistent: No such file or directory","is_error":true}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show error indicator
    assert!(
        out.contains("Exit code: non-zero") || out.contains("✗"),
        "Should show error: {}",
        out
    );
}

#[test]
fn test_bash_tool_result_empty_output() {
    let mut processor = StreamProcessor::with_options(false, true);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"bash_1","name":"Bash","input":{"command":"true"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"bash_1","content":"","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show success indicator for empty output
    assert!(
        out.contains("(ok)") || out.contains("✓"),
        "Should show success for empty output: {}",
        out
    );
}

#[test]
fn test_bash_tool_result_truncates_long_output() {
    let mut processor = StreamProcessor::with_options(false, true);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"bash_1","name":"Bash","input":{"command":"find /"}}]}}"#,
    );

    // Generate very long output (>30 lines)
    let mut lines = Vec::new();
    for i in 0..100 {
        lines.push(format!("/path/to/file_{}.txt", i));
    }
    let long_output = lines.join("\n");

    let output = processor.process_line(&format!(
        r#"{{"type":"user","message":{{"id":"user_1","content":[{{"type":"tool_result","tool_use_id":"bash_1","content":"{}","is_error":false}}]}}}}"#,
        long_output.replace('\n', "\\n")
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show truncation indicator
    assert!(
        out.contains("truncated"),
        "Should indicate truncation: {}",
        out
    );
}

#[test]
fn test_bash_tool_result_with_terminal_dimmed_styling() {
    let mut processor = StreamProcessor::with_options(true, true);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"bash_1","name":"Bash","input":{"command":"echo test"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"bash_1","content":"test","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should have dimmed styling (ANSI code 90)
    assert!(
        out.contains("\x1b[90m"),
        "Should have dimmed styling: {}",
        out
    );
}
