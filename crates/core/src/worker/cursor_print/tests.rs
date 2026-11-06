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

fn options(resume: Option<&str>) -> PrintArgs<'_> {
    PrintArgs {
        workspace: Path::new("/tmp/work"),
        model: VARIANT,
        resume,
    }
}

#[test]
fn success_yields_session_activity_and_usage() {
    let (events, outcome) = fold(SUCCESS, false);
    assert_eq!(events[0], PrintEvent::Init(SESSION.to_string()));
    assert_eq!(events[1], PrintEvent::Activity);
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
    ] {
        assert_eq!(
            decode_print_line(line, VARIANT),
            PrintEvent::Ignored,
            "{line}"
        );
    }
}

#[test]
fn argv_first_turn_has_no_resume() {
    let args = print_args(&options(None));
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
            VARIANT,
            "--force"
        ]
    );
}

#[test]
fn argv_resume_adds_chat_id() {
    let args = print_args(&options(Some(SESSION)));
    let at = args.iter().position(|arg| arg == "--resume").unwrap();
    assert_eq!(args[at + 1], SESSION);
}

#[test]
fn argv_never_carries_a_prompt() {
    let args = print_args(&options(Some(SESSION)));
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

#[test]
fn a_result_decides_the_turn_end_whatever_the_exit() {
    let failed = TurnEnd::Failed {
        message: "boom".to_string(),
        transient: false,
    };
    assert_eq!(
        turn_end_for(ExitOutcome::Ended, Some(failed.clone())),
        failed
    );
    assert_eq!(
        turn_end_for(ExitOutcome::Ended, Some(TurnEnd::Succeeded)),
        TurnEnd::Succeeded
    );
}

#[test]
fn an_interrupted_exit_ends_the_turn_interrupted() {
    assert_eq!(
        turn_end_for(ExitOutcome::Interrupted, None),
        TurnEnd::Interrupted
    );
}

#[test]
fn an_exit_without_a_result_fails_the_turn() {
    let (events, outcome) = fold(CRASH, false);
    assert!(matches!(
        turn_end_for(outcome, result_end(&events)),
        TurnEnd::Failed { .. }
    ));
}

#[test]
fn the_interrupted_fixture_ends_interrupted() {
    let (events, outcome) = fold(INTERRUPTED, true);
    assert_eq!(
        turn_end_for(outcome, result_end(&events)),
        TurnEnd::Interrupted
    );
}

#[test]
fn the_first_init_names_the_chat_and_later_ones_never_replace_it() {
    let init = |id: &str| PrintEvent::Init(id.to_string());
    assert_eq!(next_chat(None, &init("a")), Some("a".to_string()));
    assert_eq!(next_chat(Some("a"), &init("b")), Some("a".to_string()));
    assert_eq!(next_chat(None, &PrintEvent::Activity), None);
    assert_eq!(
        next_chat(Some("a"), &PrintEvent::Activity),
        Some("a".to_string())
    );
}

#[test]
fn the_first_turn_has_no_resume_and_later_turns_resume_the_chat() {
    let args = |resume| {
        print_args(&PrintArgs {
            workspace: Path::new("/w"),
            model: "m",
            resume,
        })
    };
    assert!(!args(None).contains(&"--resume".to_string()));
    let later = args(Some(SESSION));
    let at = later.iter().position(|arg| arg == "--resume").unwrap();
    assert_eq!(later[at + 1], SESSION);
}

#[test]
fn the_model_falls_back_to_auto() {
    assert_eq!(print_model(Some("composer-2")), "composer-2");
    assert_eq!(print_model(None), "auto");
}

#[test]
fn mcp_json_adds_the_server_to_an_empty_file() {
    let server = json!({"command": "ralph", "args": ["worker-mcp"]});
    let text = cursor_mcp_json(None, &server);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["mcpServers"]["ralph-worker"], server);
}

#[test]
fn mcp_json_keeps_other_servers_and_replaces_ours() {
    let server = json!({"command": "ralph", "args": ["worker-mcp"]});
    let existing = r#"{"extra": 1, "mcpServers": {"other": {"command": "x"}, "ralph-worker": {"command": "old"}}}"#;
    let parsed: serde_json::Value =
        serde_json::from_str(&cursor_mcp_json(Some(existing), &server)).unwrap();
    assert_eq!(parsed["extra"], 1);
    assert_eq!(parsed["mcpServers"]["other"], json!({"command": "x"}));
    assert_eq!(parsed["mcpServers"]["ralph-worker"], server);
}

#[test]
fn mcp_json_replaces_an_unreadable_file() {
    let server = json!({"command": "ralph", "args": []});
    for existing in ["not json", "[]", "null"] {
        let parsed: serde_json::Value =
            serde_json::from_str(&cursor_mcp_json(Some(existing), &server)).unwrap();
        assert_eq!(parsed["mcpServers"]["ralph-worker"], server, "{existing}");
    }
}

fn result_end(events: &[PrintEvent]) -> Option<TurnEnd> {
    events.iter().find_map(|event| match event {
        PrintEvent::Result { end, .. } => Some(end.clone()),
        _ => None,
    })
}

#[test]
fn a_result_usage_becomes_a_cursor_worker_ledger_row() {
    let usage =
        json!({"inputTokens":8680,"outputTokens":36,"cacheReadTokens":6016,"cacheWriteTokens":0});
    let mapped = map_print_usage(&usage, VARIANT).unwrap();
    let row = print_usage_record(
        "w1",
        Some(4),
        "2026-01-01T00:00:00Z".to_string(),
        Some(SESSION.to_string()),
        &mapped,
    );
    let value = serde_json::to_value(&row).unwrap();
    assert_eq!(value["input_tokens"], 8680);
    assert_eq!(value["cache_read_input_tokens"], 6016);
    assert_eq!(value["cache_creation_input_tokens"], 0);
    assert_eq!(value["output_tokens"], 36);
    assert_eq!(value["role"], "worker");
    assert_eq!(value["kind"], "worker");
    assert_eq!(value["worker_id"], "w1");
    assert_eq!(value["turn_id"], 4);
    assert_eq!(value["harness"], "cursor");
    assert_eq!(value["model"], VARIANT);
    assert_eq!(value["session_id"], SESSION);
    assert!(value["cost_usd"].is_null());
}
