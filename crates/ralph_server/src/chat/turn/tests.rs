use super::*;
use crate::disk_files::DiskFiles;
use ralph_core::chunk::{ChunkType, ParsedChunk};
use ralph_core::stream::blocks::{StreamCollector, StreamProcessorResult};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

#[test]
fn background_runs_use_the_executable_the_client_gave() {
    let store = std::env::temp_dir();
    let command =
        background_command(Some(Path::new("/opt/ralph/bin/ralph")), Vec::new(), &store).unwrap();
    assert_eq!(command.get_program(), "/opt/ralph/bin/ralph");
}

#[test]
fn background_runs_fail_without_an_executable() {
    let store = std::env::temp_dir();
    assert!(background_command(None, Vec::new(), &store).is_err());
}

#[test]
fn system_prompt_file_holds_the_old_argument_bytes_privately() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("system-prompt.md");
    let system = "rules\0 it's \"$HOME\"\n  ñ trailing \n";
    write_system_prompt(&path, &system.repeat(3)).unwrap();
    write_system_prompt(&path, system).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes, b"rules it's \"$HOME\"\n  \xc3\xb1 trailing \n");
    assert_eq!(bytes, strip_controls(system).as_bytes());
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

fn finished(session_id: Option<&str>, model: Option<&str>) -> StreamingSubprocessResult {
    StreamingSubprocessResult {
        exit_code: 0,
        stderr: String::new(),
        stream_result: StreamProcessorResult {
            metadata: IterationMetadata {
                session_id: session_id.map(str::to_string),
                model: model.map(str::to_string),
            },
            ..Default::default()
        },
        request_at: None,
        cost_before: None,
        logged_before: None,
    }
}

#[test]
fn turn_row_names_the_model_from_the_claude_init_line() {
    let mut processor = StreamCollector::new(false, VerboseToolsConfig::new());
    [
        r#"{"type":"system","subtype":"init","session_id":"s1","tools":["Task","Bash","mcp__chat__help"],"mcp_servers":[{"name":"chat","status":"connected"}],"model":"claude-haiku-5-5","permissionMode":"default","slash_commands":["compact"],"agents":["general-purpose"]}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0018,"usage":{"input_tokens":3,"output_tokens":5},"session_id":"s1"}"#,
    ]
    .iter()
    .for_each(|line| {
        processor.process_line(line, &DiskFiles);
    });
    let result = Ok(StreamingSubprocessResult {
        exit_code: 0,
        stderr: String::new(),
        stream_result: processor.finish(&DiskFiles).1,
        request_at: None,
        cost_before: None,
        logged_before: None,
    });
    let identity = carry_metadata(&IterationMetadata::new(), true, &turn_metadata(&result));
    let costs = &result.as_ref().unwrap().stream_result.costs;
    let row = usage_record(
        CallSite::Turn {
            trigger: Trigger::User,
        },
        "t".into(),
        costs,
        &identity,
    )
    .unwrap();
    let row = serde_json::to_value(row).unwrap();
    assert_eq!(row["model"], "claude-haiku-5-5");
    assert_eq!(row["session_id"], "s1");
}

const PROBE_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0024732699999999996,"duration_ms":4404,"session_id":"47488cc1","usage":{"input_tokens":2,"cache_creation_input_tokens":10086,"cache_read_input_tokens":6887,"output_tokens":774,"output_tokens_details":{"thinking_tokens":565},"cache_creation":{"ephemeral_1h_input_tokens":10086,"ephemeral_5m_input_tokens":0}}}"#;

fn probe_partial() -> StreamingSubprocessResult {
    let mut processor = StreamCollector::new(false, VerboseToolsConfig::new());
    processor.process_line(PROBE_RESULT, &DiskFiles);
    StreamingSubprocessResult {
        exit_code: 1,
        stderr: String::new(),
        stream_result: processor.finish(&DiskFiles).1,
        request_at: None,
        cost_before: None,
        logged_before: None,
    }
}

fn recorded_cache_writes(
    result: &Result<StreamingSubprocessResult, SubprocessError>,
) -> Option<u64> {
    recorded(result)
        .and_then(|turn| turn.stream_result.costs.usage.as_ref())
        .and_then(|usage| usage.cache_creation_input_tokens)
}

#[test]
fn a_cancelled_turn_with_a_result_still_records_its_usage() {
    let cancelled = Ok(probe_partial());
    assert_eq!(recorded_cache_writes(&cancelled), Some(10086));
}

#[test]
fn an_interrupted_or_timed_out_turn_records_its_partial_usage() {
    let interrupted = Err(SubprocessError::Interrupted {
        partial_result: Box::new(probe_partial()),
    });
    assert_eq!(recorded_cache_writes(&interrupted), Some(10086));
    let timed_out = Err(timeout(probe_partial()));
    assert_eq!(recorded_cache_writes(&timed_out), Some(10086));
    let stale = Err(SubprocessError::Stale);
    assert_eq!(recorded_cache_writes(&stale), None);
}

fn previous() -> IterationMetadata {
    IterationMetadata {
        session_id: Some("old".to_string()),
        model: Some("old-model".to_string()),
    }
}

fn timeout(partial: StreamingSubprocessResult) -> SubprocessError {
    SubprocessError::Timeout {
        timeout_secs: 1,
        partial_result: Box::new(partial),
    }
}

#[test]
fn turn_metadata_reads_a_finished_turn() {
    let result = Ok(finished(Some("s1"), Some("m1")));
    let meta = turn_metadata(&result);
    assert_eq!(meta.session_id.as_deref(), Some("s1"));
    assert_eq!(meta.model.as_deref(), Some("m1"));
}

#[test]
fn turn_metadata_reads_a_timeout_partial() {
    let result = Err(timeout(finished(Some("s2"), None)));
    let meta = turn_metadata(&result);
    assert_eq!(meta.session_id.as_deref(), Some("s2"));
    assert_eq!(meta.model, None);
}

#[test]
fn turn_metadata_reads_an_interrupted_partial() {
    let result = Err(SubprocessError::Interrupted {
        partial_result: Box::new(finished(Some("s3"), None)),
    });
    assert_eq!(turn_metadata(&result).session_id.as_deref(), Some("s3"));
}

#[test]
fn turn_metadata_is_empty_for_stale() {
    let result = Err(SubprocessError::Stale);
    assert_eq!(turn_metadata(&result), IterationMetadata::new());
}

#[test]
fn launched_stale_turn_clears_previous_identity() {
    let result = Err(SubprocessError::Stale);
    let carried = carry_metadata(&previous(), true, &turn_metadata(&result));
    assert_eq!(carried.session_id, None);
    assert_eq!(carried.model, None);
}

#[test]
fn reused_failed_turn_keeps_previous_identity() {
    let result = Err(SubprocessError::Stale);
    let carried = carry_metadata(&previous(), false, &turn_metadata(&result));
    assert_eq!(carried.session_id.as_deref(), Some("old"));
    assert_eq!(carried.model.as_deref(), Some("old-model"));
}

#[test]
fn launched_timeout_takes_partial_session_id() {
    let result = Err(timeout(finished(Some("new"), None)));
    let carried = carry_metadata(&previous(), true, &turn_metadata(&result));
    assert_eq!(carried.session_id.as_deref(), Some("new"));
    assert_eq!(carried.model, None);
}

#[test]
fn process_turns_stay_when_no_session_ran() {
    assert_eq!(next_process_turns(false, false, 4), 4);
    assert_eq!(next_process_turns(false, true, 4), 4);
}

#[test]
fn process_turns_restart_at_one_on_a_launch() {
    assert_eq!(next_process_turns(true, true, 4), 1);
}

#[test]
fn process_turns_grow_on_a_reused_session() {
    assert_eq!(next_process_turns(true, false, 4), 5);
}

#[test]
fn completed_turn_keeps_session() {
    assert!(keeps_session(&TurnEnd::Completed));
}

#[test]
fn cancelled_turn_drops_session() {
    assert!(!keeps_session(&TurnEnd::Cancelled));
}

#[test]
fn exit_keeps_session() {
    assert!(keeps_session(&TurnEnd::Exit));
}

#[test]
fn reused_session_that_is_stale_relaunches() {
    assert!(relaunch(true, &Err::<(), _>(SubprocessError::Stale)));
}

#[test]
fn fresh_session_that_is_stale_does_not_relaunch() {
    assert!(!relaunch(false, &Err::<(), _>(SubprocessError::Stale)));
}

#[test]
fn reused_session_with_leftover_result_relaunches() {
    assert!(relaunch(
        true,
        &Err::<(), _>(SubprocessError::LeftoverResult)
    ));
}

#[test]
fn fresh_session_with_leftover_result_does_not_relaunch() {
    assert!(!relaunch(
        false,
        &Err::<(), _>(SubprocessError::LeftoverResult)
    ));
}

#[test]
fn reused_session_with_other_error_does_not_relaunch() {
    assert!(!relaunch(true, &Err::<(), _>(SubprocessError::Signaled)));
}

#[test]
fn reused_session_that_succeeds_does_not_relaunch() {
    assert!(!relaunch(true, &Ok::<_, SubprocessError>(())));
}

#[test]
fn stale_error_message_names_the_exit() {
    assert_eq!(
        SubprocessError::Stale.to_string(),
        "claude process exited before the turn"
    );
}

#[test]
fn leftover_result_error_message_names_the_leftover() {
    assert_eq!(
        SubprocessError::LeftoverResult.to_string(),
        "claude process wrote a result after the last turn"
    );
}

#[test]
fn only_a_replied_turn_is_timed_ok() {
    type Outcome = Result<TurnOutcome, Box<dyn std::error::Error>>;
    let ok = |outcome: Outcome| turn_ok(&outcome);
    assert!(ok(Ok(TurnOutcome::Replied)));
    assert!(!ok(Ok(TurnOutcome::Unanswered)));
    assert!(!ok(Ok(TurnOutcome::Exit)));
    assert!(!ok(Err("store".into())));
}

#[test]
fn a_turn_row_for_a_failed_turn_keeps_the_waits_and_says_not_ok() {
    use ralph_core::chat_memory::{turn_row, TimingKey};
    let timing = TurnTiming::default()
        .with_memory_wait(Duration::from_millis(12))
        .with_outage_wait(Duration::from_millis(30));
    let key = TimingKey {
        store: "/s",
        run: Some("r"),
        at: chrono::Utc::now(),
    };
    let line = ralph_core::chat_memory::encode_timing(&turn_row(
        &key,
        Duration::from_millis(80),
        &timing,
        false,
    ))
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["memory_wait_ms"], 12);
    assert_eq!(value["outage_retries"], 1);
}

#[test]
fn only_worker_turns_log_before_the_reply() {
    assert!(logged_before_reply(Trigger::Worker));
    assert!(!logged_before_reply(Trigger::User));
}

#[test]
fn unanswered_turn_restores_the_draft() {
    assert_eq!(
        draft_after(TurnOutcome::Unanswered, "hi".into()),
        Some("hi".into())
    );
    assert_eq!(draft_after(TurnOutcome::Replied, "hi".into()), None);
    assert_eq!(draft_after(TurnOutcome::Exit, "hi".into()), None);
}

#[test]
fn rule_follows_shown_output_only() {
    assert!(!shown_output(&[]));
    assert!(!shown_output(&[OutputBlock::Separator]));
    assert!(shown_output(&[
        OutputBlock::Separator,
        OutputBlock::text(ParsedChunk {
            chunk_type: ChunkType::Prose,
            content: "x".to_string(),
        })
    ]));
}
