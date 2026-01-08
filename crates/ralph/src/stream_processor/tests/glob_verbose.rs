//! Enhanced Glob tool verbose mode tests for StreamProcessor.
//!
//! Tests that Glob tool invocations and results are rendered
//! with enhanced formatting when verbose mode is enabled.

use crate::highlight::ThemeConfig;
use crate::stream_processor::{StreamProcessor, VerboseToolsConfig};

#[test]
fn test_glob_verbose_invocation_shows_pattern() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"**/*.rs"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show the pattern in full
    assert!(out.contains("**/*.rs"), "Should show full pattern: {}", out);
    // Should show Pattern label
    assert!(
        out.contains("Pattern:"),
        "Should have Pattern label: {}",
        out
    );
}

#[test]
fn test_glob_verbose_invocation_shows_path() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"*.toml","path":"/project/crates"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show the path
    assert!(out.contains("Path:"), "Should have Path label: {}", out);
    assert!(
        out.contains("/project/crates"),
        "Should show search path: {}",
        out
    );
}

#[test]
fn test_glob_verbose_invocation_default_path() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"Cargo.toml"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show default path "."
    assert!(out.contains("Path:"), "Should have Path label: {}", out);
    // In plain text it shows "." for the default path
    assert!(out.contains("."), "Should show default path '.': {}", out);
}

#[test]
fn test_glob_verbose_result_shows_match_count() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    // First send the invocation
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"**/*.rs"}}]}}"#,
    );

    // Then send results
    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"glob_1","content":"src/main.rs\nsrc/lib.rs\ntests/integration.rs","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show match count
    assert!(
        out.contains("3 files matched"),
        "Should show file count: {}",
        out
    );
}

#[test]
fn test_glob_verbose_result_single_file_grammar() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"Cargo.toml"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"glob_1","content":"Cargo.toml","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should use singular "file"
    assert!(
        out.contains("1 file matched"),
        "Should use singular 'file': {}",
        out
    );
    assert!(!out.contains("1 files"), "Should NOT use plural: {}", out);
}

#[test]
fn test_glob_verbose_result_empty() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"nonexistent/**/*.xyz"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"glob_1","content":"","is_error":false}]}}"#,
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
fn test_glob_verbose_result_error() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"**/*.rs"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"glob_1","content":"Invalid glob pattern","is_error":true}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show error indicator
    assert!(
        out.contains("Glob error") || out.contains("!"),
        "Should show error: {}",
        out
    );
}

#[test]
fn test_glob_verbose_result_groups_by_directory() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"**/*.rs"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"glob_1","content":"src/main.rs\nsrc/lib.rs\ntests/integration.rs\nCargo.toml","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show directory groupings
    assert!(out.contains("src/"), "Should show src directory: {}", out);
    assert!(
        out.contains("tests/"),
        "Should show tests directory: {}",
        out
    );
    // Files should be grouped under their directories
    // The filenames appear under their directories
    assert!(
        out.contains("main.rs"),
        "Should show main.rs filename: {}",
        out
    );
    assert!(
        out.contains("lib.rs"),
        "Should show lib.rs filename: {}",
        out
    );
}

#[test]
fn test_glob_verbose_result_truncates_large_output() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"**/*.rs"}}]}}"#,
    );

    // Generate large output (>200 lines)
    let mut lines = Vec::new();
    for i in 0..250 {
        lines.push(format!("src/file_{}.rs", i));
    }
    let large_content = lines.join("\n");

    let output = processor.process_line(&format!(
        r#"{{"type":"user","message":{{"id":"user_1","content":[{{"type":"tool_result","tool_use_id":"glob_1","content":"{}","is_error":false}}]}}}}"#,
        large_content.replace('\n', "\\n")
    ));

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show truncation indicator
    assert!(
        out.contains("more files"),
        "Should show truncation indicator: {}",
        out
    );
}

#[test]
fn test_glob_verbose_with_terminal_highlighting() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), true, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"**/*.rs"}}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should have ANSI codes
    assert!(out.contains("\x1b["), "Should have ANSI codes: {}", out);
    // Should have cyan header
    assert!(out.contains("\x1b[36m"), "Should have cyan header: {}", out);
}

#[test]
fn test_glob_non_verbose_uses_compact_format() {
    // No verbose config - should use compact format
    let mut processor = StreamProcessor::with_options(false, true);

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"src/**/*.rs","path":"/project"}}]}}"#,
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
    assert!(out.contains("src/**/*.rs"), "Should show pattern: {}", out);
}

#[test]
fn test_glob_verbose_plain_text_format() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"**/*.toml"}}]}}"#,
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
        out.contains("> Glob"),
        "Should have plain Glob header: {}",
        out
    );
}

#[test]
fn test_glob_verbose_result_with_nested_directories() {
    let verbose_config = VerboseToolsConfig::from_arg(Some("glob"));
    let mut processor =
        StreamProcessor::with_verbose_tools(ThemeConfig::new(), false, true, verbose_config)
            .unwrap();

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"glob_1","name":"Glob","input":{"pattern":"**/*.rs"}}]}}"#,
    );

    let output = processor.process_line(
        r#"{"type":"user","message":{"id":"user_1","content":[{"type":"tool_result","tool_use_id":"glob_1","content":"src/stream_processor/mod.rs\nsrc/stream_processor/types.rs\nsrc/run/mod.rs","is_error":false}]}}"#,
    );

    assert!(output.is_some());
    let out = output.unwrap();
    // Should show nested directories
    assert!(
        out.contains("src/stream_processor/"),
        "Should show nested directory: {}",
        out
    );
    assert!(
        out.contains("src/run/"),
        "Should show other nested directory: {}",
        out
    );
}
