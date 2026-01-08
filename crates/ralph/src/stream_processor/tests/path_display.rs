//! Full path display tests for StreamProcessor.
//!
//! Tests that file paths are shown in full without truncation,
//! while other arguments are truncated as appropriate.

use crate::stream_processor::StreamProcessor;

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

// Note: Bash commands are now shown in full with syntax highlighting.
// The old test_bash_command_is_truncated has been replaced by
// test_bash_command_shown_in_full_without_truncation in the
// "Enhanced Bash Tool Rendering Tests" section.

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
