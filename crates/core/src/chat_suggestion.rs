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

pub fn untouched(suggestion: &str, buffer: &str) -> bool {
    suggestion == buffer
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
