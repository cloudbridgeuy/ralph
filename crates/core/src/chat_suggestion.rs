use crate::chat_hold::{hold_targets, HoldFilter};
use crate::chat_send_it::Step;
use crate::chat_status::Status;
use serde::Serialize;
use serde_json::{json, Value};

pub const SUGGESTION_MAX_CHARS: usize = 200;

pub fn suggest_reply_tool() -> Value {
    json!({
        "name": "suggest_reply",
        "description": "Pre-fill the user's next prompt with the words they would type. Call it before any reply text and never mention it. A later call replaces it. A reply of exactly `/hold <target>` can start that hold at once. `step` marks a routine step `/send-it` auto-sends; approval, merge, push, abandon get none.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": {"type": "string", "maxLength": SUGGESTION_MAX_CHARS, "description": "The exact reply, on one line."},
                "step": {"type": "string", "enum": Step::ALL.map(Step::as_str), "description": "The routine step this reply performs, if any."}
            },
            "required": ["text"]
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    pub step: Option<Step>,
}

fn parse_step(args: &Value) -> Result<Option<Step>, String> {
    match args.get("step") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .and_then(Step::parse)
            .map(Some)
            .ok_or_else(|| {
                let allowed = Step::ALL.map(Step::as_str).join(", ");
                format!("Unknown step {value}. Use one of: {allowed}, or omit it.")
            }),
    }
}

pub fn parse_suggestion(args: &Value) -> Result<Suggestion, String> {
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
        Ok(Suggestion {
            text: text.to_string(),
            step: parse_step(args)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prefill {
    Draft(String),
    Suggestion(String),
}

fn prefill(replied: bool, draft: Option<String>, suggestion: Option<String>) -> Option<Prefill> {
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
    AutoSent,
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

pub fn auto_hold_notice(filter: &HoldFilter) -> String {
    format!("auto-hold: {} · Ctrl-Q to type", filter.label())
}

impl SuggestionRecord {
    pub fn held(ts: String, run: &str, turn: u64, suggestion: &str) -> Self {
        Self {
            outcome: Outcome::Held,
            ..Self::new(ts, run, turn, suggestion, suggestion)
        }
    }

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
mod tests;
