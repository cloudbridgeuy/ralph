use crate::chat_hold::{hold_targets, HoldFilter};
use crate::chat_status::Status;
use serde::Serialize;
use serde_json::{json, Value};

pub const SUGGESTION_MAX_CHARS: usize = 200;

pub fn suggest_reply_tool() -> Value {
    json!({
        "name": "suggest_reply",
        "description": "Pre-fill the user's next prompt with the exact words they would type. Call it before any reply text and never mention it. One line, at most 200 characters. A later call replaces the earlier one.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": {"type": "string", "maxLength": SUGGESTION_MAX_CHARS, "description": "The exact reply, on one line."}
            },
            "required": ["text"]
        }
    })
}

pub fn parse_suggestion(args: &Value) -> Result<String, String> {
    let Some(raw) = args.get("text").and_then(Value::as_str) else {
        return Err("Missing string argument 'text'.".to_string());
    };
    let text = raw.trim();
    let length = text.chars().count();
    if text.is_empty() {
        Err("Suggested reply is empty. Pass the exact words the user would type.".to_string())
    } else if text
        .chars()
        .any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}')
    {
        Err(
            "Suggested reply has a line break or control character. Pass one line of plain text."
                .to_string(),
        )
    } else if length > SUGGESTION_MAX_CHARS {
        Err(format!(
            "Suggested reply is {length} characters; the limit is {SUGGESTION_MAX_CHARS}. Shorten it."
        ))
    } else {
        Ok(text.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prefill {
    Draft(String),
    Suggestion(String),
}

pub fn prefill(
    replied: bool,
    draft: Option<String>,
    suggestion: Option<String>,
) -> Option<Prefill> {
    match draft {
        Some(draft) => Some(Prefill::Draft(draft)),
        None if replied => suggestion.map(Prefill::Suggestion),
        None => None,
    }
}

#[derive(Debug)]
pub struct TurnEnd<'a> {
    pub auto_hold: bool,
    pub replied: bool,
    pub draft: Option<String>,
    pub suggestion: Option<String>,
    pub status: &'a Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Hold(HoldFilter),
    Prefill(Prefill),
    Nothing,
}

fn auto_hold_target(suggestion: &str, status: &Status) -> Option<HoldFilter> {
    let target = suggestion.strip_prefix("/hold ")?;
    (!target.contains(char::is_whitespace) && hold_targets(status).contains(target))
        .then(|| HoldFilter::Source(target.to_string()))
}

pub fn turn_end(end: TurnEnd) -> Next {
    let hold = end
        .suggestion
        .as_deref()
        .filter(|_| end.auto_hold && end.replied && end.draft.is_none())
        .and_then(|suggestion| auto_hold_target(suggestion, end.status));
    if let Some(filter) = hold {
        return Next::Hold(filter);
    }
    match prefill(end.replied, end.draft, end.suggestion) {
        Some(fill) => Next::Prefill(fill),
        None => Next::Nothing,
    }
}

pub fn untouched(suggestion: &str, buffer: &str) -> bool {
    suggestion == buffer
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Accepted,
    Extended,
    Discarded,
    Held,
}

pub fn outcome(suggestion: &str, sent: &str) -> Outcome {
    let sent = sent.trim();
    if sent == suggestion {
        Outcome::Accepted
    } else if sent.starts_with(suggestion) {
        Outcome::Extended
    } else {
        Outcome::Discarded
    }
}

#[derive(Debug, Serialize)]
pub struct SuggestionRecord {
    pub ts: String,
    pub run: String,
    pub turn: u64,
    pub outcome: Outcome,
    pub suggestion: String,
    pub sent: String,
}

impl SuggestionRecord {
    pub fn new(ts: String, run: &str, turn: u64, suggestion: &str, sent: &str) -> Self {
        let sent = sent.trim();
        Self {
            ts,
            run: run.to_string(),
            turn,
            outcome: outcome(suggestion, sent),
            suggestion: suggestion.to_string(),
            sent: sent.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_status::{ScheduleState, WatchState, WorkerState};
    use crate::inbox::Entry;
    use crate::worker::Report;

    fn parse(text: &str) -> Result<String, String> {
        parse_suggestion(&json!({"text": text}))
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
        assert!(description.chars().count() <= 200);
        assert!(description.contains("before any reply text"));
        assert!(description.contains("never mention it"));
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
    fn a_finished_worker_target_prefills() {
        assert_eq!(end("/hold GUZ-9"), fill("/hold GUZ-9"));
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

    #[test]
    fn a_hold_without_a_target_prefills() {
        assert_eq!(end("/hold"), fill("/hold"));
        assert_eq!(end("/hold "), fill("/hold "));
    }

    #[test]
    fn two_tokens_or_odd_spacing_prefill() {
        assert_eq!(end("/hold GUZ-1 watch:ci"), fill("/hold GUZ-1 watch:ci"));
        assert_eq!(end("/hold  GUZ-1"), fill("/hold  GUZ-1"));
        assert_eq!(end("/hold GUZ-1 "), fill("/hold GUZ-1 "));
        assert_eq!(end(" /hold GUZ-1"), fill(" /hold GUZ-1"));
        assert_eq!(end("/hold\tGUZ-1"), fill("/hold\tGUZ-1"));
        assert_eq!(end("/holdGUZ-1"), fill("/holdGUZ-1"));
    }

    #[test]
    fn auto_hold_off_prefills() {
        assert_eq!(
            end_with(false, true, None, "/hold GUZ-1"),
            fill("/hold GUZ-1")
        );
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
    fn the_held_outcome_serializes_as_held() {
        assert_eq!(serde_json::to_value(Outcome::Held).unwrap(), json!("held"));
        assert_eq!(serde_json::to_string(&Outcome::Held).unwrap(), "\"held\"");
    }
}
