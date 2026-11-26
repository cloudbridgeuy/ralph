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
        cost_before: None,
        logged_before: None,
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
        codex_model: None,
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
fn a_timed_out_curator_call_with_a_result_gives_a_nap_and_a_subagent_row() {
    let outcome = curator_outcome(Some(CURATOR_RESULT), timed_out);
    assert_eq!(curator_row_kinds(&outcome), vec!["nap", "subagent"]);
}

#[test]
fn an_interrupted_curator_call_with_a_result_gives_a_nap_and_a_subagent_row() {
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

#[test]
fn the_cli_accepts_claude_as_a_verify_agent() {
    for agent in ["cursor", "codex", "claude", "opencode", "claude-tmux"] {
        assert!(Cli::try_parse_from(["ralph", "worker", "verify", agent]).is_ok());
    }
    for agent in ["tmux", "claude-headless"] {
        assert!(Cli::try_parse_from(["ralph", "worker", "verify", agent]).is_err());
    }
}

#[test]
fn the_hidden_drift_flags_come_as_a_pair_and_never_with_live() {
    let parse = |extra: &[&str]| {
        let mut args = vec!["ralph", "worker", "verify", "codex"];
        args.extend_from_slice(extra);
        Cli::try_parse_from(args).is_ok()
    };
    assert!(parse(&["--drift", "0.170.0", "--inbox", "/s/inbox"]));
    assert!(!parse(&["--drift", "0.170.0"]));
    assert!(!parse(&["--inbox", "/s/inbox"]));
    assert!(!parse(&[
        "--live", "--drift", "0.170.0", "--inbox", "/s/inbox"
    ]));
}

#[test]
fn codex_curator_rows_preserve_model_and_do_not_invent_cost() {
    let outcome = curator_outcome(Some(CURATOR_RESULT), timed_out);
    let done = subprocess::recorded(&outcome).unwrap();
    for model in ["gpt-5.4", "unknown-model"] {
        let rows = CuratorRows {
            kind: ralph_core::chat_memory::CallSite::Nap,
            run: None,
            store: "/store".into(),
            at: "t".into(),
            codex_model: Some(model.into()),
        };
        let value = serde_json::to_value(&curator_rows(&rows, done)[0]).unwrap();
        assert_eq!(value["harness"], "codex");
        assert_eq!(value["model"], model);
        assert!(value["cost_usd"].is_null());
        assert_eq!(value["per_thread_usage"], true);
    }
}

fn codex_final_text(text: &str) -> String {
    let mut stream = ralph_core::worker::ExecStream::default();
    stream.observe(
        &serde_json::json!({"type":"item.completed","item":{"type":"agent_message","text":text}})
            .to_string(),
    );
    stream.observe(r#"{"type":"turn.completed","usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0}}"#);
    let (result, error) = stream.finish();
    assert!(error.is_none());
    stream_processor::extract_response_text(&result.output_blocks).unwrap()
}

#[test]
fn codex_final_text_reaches_nap_merge_and_tag_parsers() {
    let nap = codex_final_text("```markdown\nA useful summary\n```");
    assert_eq!(
        ralph_core::chat_memory::nap_summary(0, Some(&nap)).unwrap(),
        "A useful summary"
    );
    let merge = codex_final_text("A merged summary #auth:1");
    let merged = ralph_core::chat_memory::nap_summary(0, Some(&merge)).unwrap();
    assert_eq!(
        ralph_core::chat_thread::check_digest(
            &merged,
            &ralph_core::chat_thread::ThreadName::parse("auth").unwrap()
        )
        .unwrap(),
        "A merged summary #auth:1"
    );
    let text = codex_final_text(r#"[{"n":1,"rule":true,"decision":false,"approval":false}]"#);
    let run = MemoryRun {
        code: 0,
        text: Some(text),
        failure: None,
    };
    let reply = memory_tag_reply(&run, true, false).unwrap();
    assert_eq!(
        ralph_core::chat_memory::parse_reply(&reply.text, &[1])
            .tagged
            .len(),
        1
    );
}

#[test]
fn malformed_codex_tag_payload_aborts_before_entry_failures() {
    for text in ["not json", "{}", "[broken"] {
        let run = MemoryRun {
            code: 0,
            text: Some(codex_final_text(text)),
            failure: None,
        };
        assert!(memory_tag_reply(&run, true, false).is_err());
        assert!(memory_tag_reply(&run, false, false).is_ok());
    }
}

#[test]
fn curator_rows_agy_has_harness_agy_and_no_cost() {
    let rows = CuratorRows {
        kind: ralph_core::chat_memory::CallSite::Nap,
        run: Some("run-1".into()),
        store: "/store".into(),
        at: "t".into(),
        codex_model: None,
    };
    let raw = ralph_core::worker::AgyUsage {
        total: 120,
        input: 100,
        output: 20,
        read: None,
        write: None,
        thought: None,
    };
    let built = agy_curator_rows(&rows, "gemini-3-flash", &raw).unwrap();
    let value = serde_json::to_value(&built[0]).unwrap();
    assert_eq!(value["harness"], "agy");
    assert_eq!(value["model"], "gemini-3-flash");
    assert_eq!(value["input_tokens"], 100);
    assert_eq!(value["run"], "run-1");
    assert!(value["cost_usd"].is_null());
}

#[test]
fn agy_curator_rows_skip_empty_usage() {
    let rows = CuratorRows {
        kind: ralph_core::chat_memory::CallSite::Nap,
        run: None,
        store: "/store".into(),
        at: "t".into(),
        codex_model: None,
    };
    let raw = ralph_core::worker::AgyUsage {
        total: 0,
        input: 0,
        output: 0,
        read: None,
        write: None,
        thought: None,
    };
    assert!(agy_curator_rows(&rows, "m", &raw).unwrap().is_empty());
}
