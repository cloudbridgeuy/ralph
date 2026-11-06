use super::*;
use crate::wire::usage::{CacheTier, CacheWrite};
use serde_json::json;
use std::path::Path;

const SUCCESS: &str = include_str!("fixtures/success.jsonl");
const TOOL: &str = include_str!("fixtures/tool.jsonl");
const INTERRUPTED: &str = include_str!("fixtures/interrupted.jsonl");
const CRASH: &str = include_str!("fixtures/crash.jsonl");
const VARIANT: &str = "composer-2";
const SESSION: &str = "2f0c6c1e-6a51-4c1f-9d57-0b3a8e1f4c21";

fn decode(text: &str) -> Vec<PrintEvent> {
    text.lines()
        .map(|line| decode_print_line(line, VARIANT))
        .collect()
}

fn fold(text: &str, sigint_sent: bool) -> (Vec<PrintEvent>, ExitOutcome) {
    let events = decode(text);
    let saw_result = events
        .iter()
        .any(|event| matches!(event, PrintEvent::Result { .. }));
    (events, exit_outcome(saw_result, sigint_sent))
}

fn expected_usage() -> ModelUsage {
    ModelUsage {
        model: VARIANT.to_string(),
        usage: Usage {
            input_uncached: 8680,
            cache_read: 6016,
            cache_writes: vec![],
            output: 36,
            reasoning: None,
            other: vec![],
        },
        long_prompt: false,
    }
}

fn options<'a>(resume: Option<&'a str>, mode: PermissionMode) -> PrintArgs<'a> {
    PrintArgs {
        workspace: Path::new("/tmp/work"),
        model: VARIANT,
        resume,
        mode,
    }
}

#[test]
fn success_yields_session_text_and_usage() {
    let (events, outcome) = fold(SUCCESS, false);
    assert_eq!(events[0], PrintEvent::Init(SESSION.to_string()));
    assert_eq!(
        events[1],
        PrintEvent::Text("Hello from the stub agent.".to_string())
    );
    assert_eq!(
        events[2],
        PrintEvent::Result {
            end: TurnEnd::Succeeded,
            usage: Some(expected_usage())
        }
    );
    assert_eq!(outcome, ExitOutcome::Ended);
}

#[test]
fn tool_calls_are_activity() {
    let (events, outcome) = fold(TOOL, false);
    assert_eq!(events[1], PrintEvent::Activity);
    assert_eq!(events[2], PrintEvent::Activity);
    assert_eq!(outcome, ExitOutcome::Ended);
}

#[test]
fn interrupted_turn_has_no_result() {
    let (events, outcome) = fold(INTERRUPTED, true);
    assert!(events
        .iter()
        .all(|event| !matches!(event, PrintEvent::Result { .. })));
    assert_eq!(outcome, ExitOutcome::Interrupted);
}

#[test]
fn crash_is_process_gone() {
    let (events, outcome) = fold(CRASH, false);
    assert_eq!(events[0], PrintEvent::Init(SESSION.to_string()));
    assert_eq!(outcome, ExitOutcome::ProcessGone);
}

#[test]
fn result_wins_over_sigint() {
    assert_eq!(exit_outcome(true, true), ExitOutcome::Ended);
}

#[test]
fn error_result_fails_the_turn() {
    let line = r#"{"type":"result","subtype":"error","is_error":true,"result":"boom"}"#;
    assert_eq!(
        decode_print_line(line, VARIANT),
        PrintEvent::Result {
            end: TurnEnd::Failed {
                message: "boom".to_string(),
                transient: false
            },
            usage: None
        }
    );
}

#[test]
fn malformed_and_unknown_lines_are_ignored() {
    for line in [
        "",
        "not json",
        "{",
        "[]",
        "{\"type\":\"mystery\"}",
        "{\"type\":\"system\",\"subtype\":\"other\"}",
        "{\"type\":\"system\",\"subtype\":\"init\"}",
        "{\"type\":\"assistant\",\"message\":{\"content\":[]}}",
    ] {
        assert_eq!(
            decode_print_line(line, VARIANT),
            PrintEvent::Ignored,
            "{line}"
        );
    }
}

#[test]
fn argv_first_turn_has_no_resume_or_force() {
    let args = print_args(&options(None, PermissionMode::Ask));
    assert_eq!(
        args,
        [
            "-p",
            "--output-format",
            "stream-json",
            "--trust",
            "--approve-mcps",
            "--workspace",
            "/tmp/work",
            "--model",
            VARIANT
        ]
    );
}

#[test]
fn argv_resume_adds_chat_id() {
    let args = print_args(&options(Some(SESSION), PermissionMode::Ask));
    let at = args.iter().position(|arg| arg == "--resume").unwrap();
    assert_eq!(args[at + 1], SESSION);
    assert!(!args.contains(&"--force".to_string()));
}

#[test]
fn argv_force_only_for_yolo() {
    let yolo = print_args(&options(None, PermissionMode::Yolo));
    assert_eq!(yolo.last().map(String::as_str), Some("--force"));
    let ask = print_args(&options(None, PermissionMode::Ask));
    assert!(!ask.contains(&"--force".to_string()));
}

#[test]
fn argv_never_carries_a_prompt() {
    let args = print_args(&options(Some(SESSION), PermissionMode::Yolo));
    assert!(args.iter().all(|arg| arg.starts_with("--")
        || arg == "-p"
        || arg == "stream-json"
        || arg == "/tmp/work"
        || arg == VARIANT
        || arg == SESSION));
}

#[test]
fn usage_maps_the_four_counters() {
    let usage = json!({"inputTokens":1,"outputTokens":2,"cacheReadTokens":3,"cacheWriteTokens":4});
    let mapped = map_print_usage(&usage, VARIANT).unwrap();
    assert_eq!(mapped.usage.input_uncached, 1);
    assert_eq!(mapped.usage.output, 2);
    assert_eq!(mapped.usage.cache_read, 3);
    assert_eq!(
        mapped.usage.cache_writes,
        [CacheWrite {
            tier: CacheTier::Unspecified,
            tokens: 4
        }]
    );
    assert_eq!(mapped.model, VARIANT);
}

#[test]
fn missing_usage_keys_give_none() {
    for usage in [
        json!(null),
        json!({}),
        json!({"inputTokens":1,"outputTokens":2,"cacheReadTokens":3}),
        json!({"inputTokens":"x","outputTokens":2,"cacheReadTokens":3,"cacheWriteTokens":0}),
    ] {
        assert_eq!(map_print_usage(&usage, VARIANT), None, "{usage}");
    }
}
