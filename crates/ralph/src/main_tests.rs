//! Tests for main.rs functions.

use super::*;

// Tests for resolve_ask_prompt

#[test]
fn test_resolve_ask_prompt_inline() {
    // Inline prompt should work
    let result = resolve_ask_prompt(Some("hello world"));
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "hello world");
}

#[test]
fn test_resolve_ask_prompt_inline_with_whitespace() {
    // Whitespace is preserved in result but validated as non-empty
    let result = resolve_ask_prompt(Some("  hello world  "));
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "  hello world  ");
}

#[test]
fn test_resolve_ask_prompt_empty_string_error() {
    // Empty string should error
    let result = resolve_ask_prompt(Some(""));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("cannot be empty"));
}

#[test]
fn test_resolve_ask_prompt_whitespace_only_error() {
    // Whitespace-only should error
    let result = resolve_ask_prompt(Some("   \n\t  "));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("cannot be empty"));
}

#[test]
fn test_resolve_ask_prompt_from_file() {
    // Read from existing file (Cargo.toml has content)
    let result = resolve_ask_prompt(Some(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")));
    assert!(result.is_ok());
    let content = result.unwrap();
    assert!(content.contains("[package]")); // Cargo.toml starts with [package]
}

// Note: Testing stdin behavior (None argument, "-" argument) is difficult in unit tests
// because stdin detection with is_terminal() returns false in test runners, making
// the behavior unpredictable. Integration tests with PTY would be needed.

const CURATOR_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.002,"duration_ms":900,"session_id":"s1","usage":{"input_tokens":3,"output_tokens":41},"modelUsage":{"claude-sonnet-5-5":{"inputTokens":2,"outputTokens":9,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"costUSD":0.001,"thinkingTokens":0}}}"#;

fn curator_outcome(
    line: Option<&str>,
    wrap: fn(Box<subprocess::StreamingSubprocessResult>) -> subprocess::SubprocessError,
) -> Result<subprocess::StreamingSubprocessResult, subprocess::SubprocessError> {
    let mut processor = stream_processor::StreamProcessor::new();
    if let Some(line) = line {
        processor.process_line(line);
    }
    Err(wrap(Box::new(subprocess::StreamingSubprocessResult {
        exit_code: -1,
        stderr: String::new(),
        stream_result: processor.finish(),
        request_at: None,
    })))
}

fn timed_out(partial: Box<subprocess::StreamingSubprocessResult>) -> subprocess::SubprocessError {
    subprocess::SubprocessError::Timeout {
        timeout_secs: 1,
        partial_result: partial,
    }
}

fn interrupted(partial: Box<subprocess::StreamingSubprocessResult>) -> subprocess::SubprocessError {
    subprocess::SubprocessError::Interrupted {
        partial_result: partial,
    }
}

fn curator_row_values(
    outcome: &Result<subprocess::StreamingSubprocessResult, subprocess::SubprocessError>,
) -> Vec<serde_json::Value> {
    let curator = CuratorRows {
        kind: ralph_core::chat_memory::CallSite::Nap,
        run: Some("run-1".into()),
        store: "/store".into(),
        at: "t".into(),
    };
    subprocess::recorded(outcome)
        .map(|done| curator_rows(&curator, done))
        .unwrap_or_default()
        .iter()
        .map(|row| serde_json::to_value(row).unwrap())
        .collect()
}

fn curator_row_kinds(
    outcome: &Result<subprocess::StreamingSubprocessResult, subprocess::SubprocessError>,
) -> Vec<String> {
    curator_row_values(outcome)
        .iter()
        .map(|row| row["kind"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_timed_out_curator_call_with_a_result_gives_a_nap_row() {
    let outcome = curator_outcome(Some(CURATOR_RESULT), timed_out);
    assert_eq!(curator_row_kinds(&outcome), vec!["nap", "subagent"]);
}

#[test]
fn an_interrupted_curator_call_with_a_result_gives_a_nap_row() {
    let outcome = curator_outcome(Some(CURATOR_RESULT), interrupted);
    assert_eq!(curator_row_kinds(&outcome), vec!["nap", "subagent"]);
}

#[test]
fn a_stopped_curator_call_without_a_result_gives_no_row() {
    let outcome = curator_outcome(None, timed_out);
    assert!(curator_row_kinds(&outcome).is_empty());
}

#[test]
fn every_curator_row_carries_the_run_and_the_store() {
    let outcome = curator_outcome(Some(CURATOR_RESULT), timed_out);
    let rows = curator_row_values(&outcome);
    assert_eq!(rows.len(), 2);
    rows.iter().for_each(|row| {
        assert_eq!(row["run"], "run-1");
        assert_eq!(row["store"], "/store");
    });
    assert_eq!(rows[1]["model"], "claude-sonnet-5-5");
}
