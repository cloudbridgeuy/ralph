use super::*;
use crate::chat_status::{ScheduleState, WatchState, WorkerState};
use crate::inbox::Entry;
use crate::worker::Report;

fn parse(text: &str) -> Result<String, String> {
    parse_suggestion(&json!({"text": text})).map(|suggestion| suggestion.text)
}

fn parse_step(step: Value) -> Result<Suggestion, String> {
    parse_suggestion(&json!({"text": "go on", "step": step}))
}

#[test]
fn plain_text_is_kept_and_trimmed() {
    assert_eq!(parse("  yes, go ahead \n"), Ok("yes, go ahead".to_string()));
}

#[test]
fn empty_and_whitespace_are_rejected() {
    assert!(parse("").is_err());
    assert!(parse(" \t \n ").is_err());
}

#[test]
fn inner_line_breaks_are_rejected() {
    assert!(parse("one\ntwo").unwrap_err().contains("line break"));
    assert!(parse("one\rtwo").unwrap_err().contains("line break"));
}

#[test]
fn unicode_separators_and_control_characters_are_rejected() {
    assert!(parse("one\u{2028}two").is_err());
    assert!(parse("one\u{2029}two").is_err());
    assert!(parse("one\u{0085}two").is_err());
    assert!(parse("one\ttwo").is_err());
}

#[test]
fn length_limit_counts_characters_and_never_truncates() {
    assert_eq!(parse(&"a".repeat(200)).map(|t| t.len()), Ok(200));
    let error = parse(&"a".repeat(201)).unwrap_err();
    assert!(error.contains("201") && error.contains("200"));
    assert_eq!(parse(&"é".repeat(200)).map(|t| t.chars().count()), Ok(200));
    assert!(parse(&"é".repeat(201)).is_err());
}

#[test]
fn missing_or_non_string_text_is_an_error() {
    assert!(parse_suggestion(&json!({})).is_err());
    assert!(parse_suggestion(&json!({"text": 5})).is_err());
    assert!(parse_suggestion(&json!({"text": null})).is_err());
    assert!(parse_suggestion(&json!({"other": "x"})).is_err());
}

#[test]
fn tool_schema_names_the_tool_and_requires_text() {
    let tool = suggest_reply_tool();
    assert_eq!(tool["name"], "suggest_reply");
    assert_eq!(tool["inputSchema"]["required"], json!(["text"]));
    assert_eq!(tool["inputSchema"]["properties"]["text"]["maxLength"], 200);
    let description = tool["description"].as_str().unwrap();
    assert!(description.chars().count() <= 300);
    assert!(description.contains("before any reply text"));
    assert!(description.contains("never mention it"));
    assert!(description.contains("`/hold <target>` can start that hold at once"));
}

fn some(text: &str) -> Option<String> {
    Some(text.to_string())
}

fn draft(text: &str) -> Option<Prefill> {
    Some(Prefill::Draft(text.to_string()))
}

fn suggestion(text: &str) -> Option<Prefill> {
    Some(Prefill::Suggestion(text.to_string()))
}

#[test]
fn a_draft_always_wins() {
    assert_eq!(prefill(true, some("d"), some("s")), draft("d"));
    assert_eq!(prefill(false, some("d"), some("s")), draft("d"));
    assert_eq!(prefill(true, some("d"), None), draft("d"));
    assert_eq!(prefill(false, some("d"), None), draft("d"));
}

#[test]
fn a_suggestion_fills_only_after_a_reply_without_a_draft() {
    assert_eq!(prefill(true, None, some("s")), suggestion("s"));
    assert_eq!(prefill(false, None, some("s")), None);
}

#[test]
fn nothing_in_nothing_out() {
    assert_eq!(prefill(true, None, None), None);
    assert_eq!(prefill(false, None, None), None);
}

#[test]
fn untouched_needs_exact_equality() {
    assert!(untouched("yes", "yes"));
    assert!(untouched("", ""));
    assert!(!untouched("yes", "ye"));
    assert!(!untouched("yes", "yes "));
    assert!(!untouched("yes", "Yes"));
    assert!(!untouched("yes", ""));
}

#[test]
fn identical_send_is_accepted() {
    assert_eq!(outcome("yes", "yes"), Outcome::Accepted);
}

#[test]
fn longer_send_with_the_suggestion_as_prefix_is_extended() {
    assert_eq!(outcome("yes", "yes, and more"), Outcome::Extended);
}

#[test]
fn replaced_text_is_discarded() {
    assert_eq!(outcome("yes", "no way"), Outcome::Discarded);
}

#[test]
fn surrounding_whitespace_is_trimmed_before_comparing() {
    assert_eq!(outcome("yes", "  yes \n"), Outcome::Accepted);
    assert_eq!(outcome("yes", " yes please "), Outcome::Extended);
}

#[test]
fn empty_send_is_discarded() {
    assert_eq!(outcome("yes", ""), Outcome::Discarded);
    assert_eq!(outcome("yes", "   "), Outcome::Discarded);
}

#[test]
fn case_only_change_is_discarded() {
    assert_eq!(outcome("yes", "Yes"), Outcome::Discarded);
}

#[test]
fn accepted_command_counts_as_accepted() {
    assert_eq!(outcome("/help", "/help"), Outcome::Accepted);
}

#[test]
fn shorter_prefix_of_the_suggestion_is_discarded() {
    assert_eq!(outcome("yes please", "yes"), Outcome::Discarded);
}

#[test]
fn record_trims_sent_and_serializes_exact_keys() {
    let record = SuggestionRecord::new(
        "2026-01-01T00:00:00+00:00".to_string(),
        "7-t",
        7,
        "yes",
        " yes ok ",
    );
    assert_eq!(
        serde_json::to_value(&record).unwrap(),
        json!({
            "ts": "2026-01-01T00:00:00+00:00",
            "run": "7-t",
            "turn": 7,
            "outcome": "extended",
            "suggestion": "yes",
            "sent": "yes ok"
        })
    );
    assert_eq!(
        serde_json::to_string(&record).unwrap(),
        r#"{"ts":"2026-01-01T00:00:00+00:00","run":"7-t","turn":7,"outcome":"extended","suggestion":"yes","sent":"yes ok"}"#
    );
}

fn running(id: &str) -> WorkerState {
    WorkerState {
        id: id.to_string(),
        state: "running".to_string(),
    }
}

fn status() -> Status {
    Status {
        workers: vec![
            running("GUZ-1"),
            WorkerState {
                id: "GUZ-9".to_string(),
                state: "done".to_string(),
            },
            WorkerState {
                id: "GUZ-5".to_string(),
                state: "idle".to_string(),
            },
        ],
        watches: vec![WatchState {
            id: "watch:ci".to_string(),
            left: None,
        }],
        schedule: vec![ScheduleState {
            id: "nightly".to_string(),
            time: "09:00".to_string(),
        }],
        ..Status::default()
    }
}

fn end(suggestion: &str) -> Next {
    end_with(true, true, None, suggestion)
}

fn end_with(auto_hold: bool, replied: bool, draft: Option<&str>, suggestion: &str) -> Next {
    turn_end(TurnEnd {
        auto_hold,
        replied,
        draft: draft.map(str::to_string),
        suggestion: some(suggestion),
        status: &status(),
    })
}

fn hold(target: &str) -> Next {
    Next::Hold(HoldFilter::Source(target.to_string()))
}

fn fill(text: &str) -> Next {
    Next::Prefill(Prefill::Suggestion(text.to_string()))
}

#[test]
fn a_running_worker_target_holds() {
    assert_eq!(end("/hold GUZ-1"), hold("GUZ-1"));
}

#[test]
fn a_done_worker_target_holds() {
    assert_eq!(end("/hold GUZ-9"), hold("GUZ-9"));
}

#[test]
fn an_idle_worker_target_holds() {
    assert_eq!(end("/hold GUZ-5"), hold("GUZ-5"));
}

#[test]
fn a_watch_target_holds() {
    assert_eq!(end("/hold watch:ci"), hold("watch:ci"));
}

#[test]
fn a_schedule_target_holds_and_matches_the_task_reports() {
    assert_eq!(end("/hold schedule:nightly"), hold("schedule:nightly"));
    let Next::Hold(filter) = end("/hold schedule:nightly") else {
        panic!("expected a hold");
    };
    for worker in ["schedule:nightly", "sched-nightly"] {
        let entry = Entry {
            source: "s".to_string(),
            report: Report {
                worker: worker.to_string(),
                at: "t".to_string(),
                status: "done".to_string(),
                text: String::new(),
            },
        };
        assert!(filter.matches(&entry));
    }
    assert_eq!(end("/hold nightly"), fill("/hold nightly"));
}

#[test]
fn an_unknown_target_prefills() {
    assert_eq!(end("/hold nobody"), fill("/hold nobody"));
}

fn set(targets: &[&str]) -> Next {
    Next::Hold(HoldFilter::Set(
        targets.iter().map(|target| target.to_string()).collect(),
    ))
}

#[test]
fn a_bare_hold_holds_for_anyone_when_there_are_targets() {
    assert_eq!(end("/hold"), Next::Hold(HoldFilter::Any));
}

#[test]
fn a_bare_hold_prefills_without_targets() {
    let next = turn_end(TurnEnd {
        auto_hold: true,
        replied: true,
        draft: None,
        suggestion: some("/hold"),
        status: &Status::default(),
    });
    assert_eq!(next, fill("/hold"));
}

#[test]
fn listed_targets_hold_as_a_set_in_the_order_given() {
    assert_eq!(end("/hold GUZ-1 watch:ci"), set(&["GUZ-1", "watch:ci"]));
    assert_eq!(
        end("/hold schedule:nightly GUZ-9 GUZ-1"),
        set(&["schedule:nightly", "GUZ-9", "GUZ-1"])
    );
}

#[test]
fn one_unknown_target_prefills() {
    assert_eq!(end("/hold GUZ-1 nobody"), fill("/hold GUZ-1 nobody"));
    assert_eq!(end("/hold nobody GUZ-1"), fill("/hold nobody GUZ-1"));
}

#[test]
fn non_canonical_spacing_case_or_duplicates_prefill() {
    for text in [
        "/hold ",
        "/hold  GUZ-1",
        "/hold GUZ-1 ",
        " /hold GUZ-1",
        "/hold\tGUZ-1",
        "/hold GUZ-1  watch:ci",
        "/hold GUZ-1\twatch:ci",
        "/hold GUZ-1 GUZ-1",
        "/hold GUZ-1 watch:ci GUZ-1",
        "/hold guz-1",
        "/holdGUZ-1",
        "/hold\n",
    ] {
        assert_eq!(end(text), fill(text), "{text:?}");
    }
}

#[test]
fn auto_hold_off_prefills() {
    for text in ["/hold GUZ-1", "/hold", "/hold GUZ-1 watch:ci"] {
        assert_eq!(end_with(false, true, None, text), fill(text));
    }
}

#[test]
fn without_a_reply_nothing_happens() {
    assert_eq!(end_with(true, false, None, "/hold GUZ-1"), Next::Nothing);
}

#[test]
fn a_draft_beats_a_hold() {
    assert_eq!(
        end_with(true, true, Some("d"), "/hold GUZ-1"),
        Next::Prefill(Prefill::Draft("d".to_string()))
    );
}

#[test]
fn a_plain_suggestion_prefills() {
    assert_eq!(end("yes"), fill("yes"));
}

#[test]
fn no_suggestion_means_nothing() {
    let next = turn_end(TurnEnd {
        auto_hold: true,
        replied: true,
        draft: None,
        suggestion: None,
        status: &status(),
    });
    assert_eq!(next, Next::Nothing);
}

#[test]
fn a_known_step_is_parsed() {
    let suggestion = parse_step(json!("fix_round")).unwrap();
    assert_eq!(suggestion.step, Some(Step::FixRound));
    assert_eq!(suggestion.text, "go on");
}

#[test]
fn no_step_or_a_null_step_means_none() {
    assert_eq!(
        parse_suggestion(&json!({"text": "go on"})).unwrap().step,
        None
    );
    assert_eq!(parse_step(Value::Null).unwrap().step, None);
}

#[test]
fn an_unknown_or_non_string_step_lists_the_allowed_values() {
    for bad in [json!("push"), json!(3)] {
        let error = parse_step(bad).unwrap_err();
        for step in Step::ALL {
            assert!(error.contains(step.as_str()), "{error}");
        }
    }
}

#[test]
fn the_schema_offers_the_step_enum_as_optional() {
    let tool = suggest_reply_tool();
    assert_eq!(
        tool["inputSchema"]["properties"]["step"]["enum"],
        json!([
            "fix_round",
            "live_check",
            "checks",
            "reinstall",
            "done_cleanup"
        ])
    );
    assert_eq!(tool["inputSchema"]["required"], json!(["text"]));
    let description = tool["description"].as_str().unwrap();
    assert!(description.contains("/send-it"));
}

#[test]
fn the_auto_sent_outcome_serializes_as_autosent() {
    assert_eq!(
        serde_json::to_value(Outcome::AutoSent).unwrap(),
        json!("autosent")
    );
}

#[test]
fn the_held_outcome_serializes_as_held() {
    assert_eq!(serde_json::to_value(Outcome::Held).unwrap(), json!("held"));
    assert_eq!(serde_json::to_string(&Outcome::Held).unwrap(), "\"held\"");
}

#[test]
fn a_held_record_has_the_held_outcome_and_the_suggestion_as_sent() {
    let record = SuggestionRecord::held(
        "2026-01-01T00:00:00+00:00".to_string(),
        "7-t",
        7,
        "/hold GUZ-1",
    );
    assert_eq!(
        serde_json::to_value(&record).unwrap(),
        json!({
            "ts": "2026-01-01T00:00:00+00:00",
            "run": "7-t",
            "turn": 7,
            "outcome": "held",
            "suggestion": "/hold GUZ-1",
            "sent": "/hold GUZ-1"
        })
    );
}

#[test]
fn the_notice_names_the_target_and_the_way_out() {
    assert_eq!(
        auto_hold_notice(&HoldFilter::Source("GUZ-1".to_string())),
        "auto-hold: GUZ-1 · Ctrl-Q to type"
    );
}

#[test]
fn the_notice_names_a_set_or_anyone() {
    assert_eq!(
        auto_hold_notice(&HoldFilter::Set(vec!["a".to_string(), "b".to_string()])),
        "auto-hold: a, b · Ctrl-Q to type"
    );
    assert_eq!(
        auto_hold_notice(&HoldFilter::Any),
        "auto-hold: anyone · Ctrl-Q to type"
    );
}

#[test]
fn an_auto_sent_record_has_the_auto_sent_outcome_and_the_sent_line() {
    let record = SuggestionRecord::auto_sent(
        "2026-01-01T00:00:00+00:00".to_string(),
        "7-t",
        7,
        "run the checks",
    );
    assert_eq!(
        serde_json::to_value(&record).unwrap(),
        json!({
            "ts": "2026-01-01T00:00:00+00:00",
            "run": "7-t",
            "turn": 7,
            "outcome": "autosent",
            "suggestion": "run the checks",
            "sent": "[send-it] run the checks"
        })
    );
}
