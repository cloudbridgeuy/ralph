use crate::mcp::string_arg;
use crate::sanitize::strip_controls;
use chrono::DateTime;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const STATUS_DIR: &str = "status";
pub const STATUS_MAX_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub at: String,
    pub text: String,
}

pub fn status_path(store: &Path, id: &str) -> PathBuf {
    store.join(STATUS_DIR).join(id)
}

pub fn status_tool() -> Value {
    json!({
        "name": "set_status",
        "description": "Set your one-sentence status. State in one sentence what you are doing now; update it when that changes. It is not a report: it costs no turn and the person does not get a message.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": {"type": "string", "maxLength": STATUS_MAX_CHARS, "description": "One sentence: what you are doing now."}
            },
            "required": ["text"]
        }
    })
}

pub fn parse_status(args: &Value) -> Result<String, String> {
    let cleaned = strip_controls(&string_arg(args, "text")?).replace(['\n', '\t'], " ");
    let text = cleaned.trim();
    let length = text.chars().count();
    if text.is_empty() {
        Err("Status text is empty.".to_string())
    } else if length > STATUS_MAX_CHARS {
        Err(format!(
            "Status is {length} characters; the limit is {STATUS_MAX_CHARS}. Shorten it to one sentence."
        ))
    } else {
        Ok(text.to_string())
    }
}

pub fn encode_status(at: &str, text: &str) -> String {
    format!("{at}\n{text}\n")
}

pub fn decode_status(raw: &str) -> Option<StatusLine> {
    let (at, rest) = raw.split_once('\n')?;
    DateTime::parse_from_rfc3339(at).ok()?;
    let text = rest.trim();
    (!text.is_empty()).then(|| StatusLine {
        at: at.to_string(),
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Value {
        json!({"text": text})
    }

    #[test]
    fn a_valid_status_parses_trimmed() {
        assert_eq!(
            parse_status(&args("  Reading the tests. ")),
            Ok("Reading the tests.".to_string())
        );
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        assert!(parse_status(&args(&"é".repeat(STATUS_MAX_CHARS))).is_ok());
        assert!(parse_status(&args(&"a".repeat(STATUS_MAX_CHARS))).is_ok());
        let err = parse_status(&args(&"a".repeat(STATUS_MAX_CHARS + 1))).unwrap_err();
        assert!(err.contains("121") && err.contains("120"), "{err}");
    }

    #[test]
    fn empty_and_blank_statuses_are_rejected() {
        for blank in ["", "   ", "\n\t", "\0\u{1}"] {
            assert!(parse_status(&args(blank)).is_err(), "{blank:?}");
        }
    }

    #[test]
    fn controls_are_stripped_and_newlines_become_spaces() {
        assert_eq!(
            parse_status(&args("a\0b\u{1}c\r\nd\te\u{7f}")),
            Ok("abc d e".to_string())
        );
    }

    #[test]
    fn missing_or_non_string_text_is_an_error() {
        assert!(parse_status(&json!({})).is_err());
        assert!(parse_status(&json!({"text": 5})).is_err());
    }

    #[test]
    fn the_tool_requires_text_and_caps_its_length() {
        let tool = status_tool();
        assert_eq!(tool["name"], "set_status");
        assert_eq!(tool["inputSchema"]["required"], json!(["text"]));
        assert_eq!(tool["inputSchema"]["properties"]["text"]["maxLength"], 120);
    }

    #[test]
    fn encode_and_decode_round_trip() {
        let at = "2026-01-01T00:00:00+00:00";
        assert_eq!(
            decode_status(&encode_status(at, "Fixing the build.")),
            Some(StatusLine {
                at: at.into(),
                text: "Fixing the build.".into()
            })
        );
    }

    #[test]
    fn garbage_does_not_decode() {
        for raw in [
            "",
            "text only",
            "not a time\nhello\n",
            "2026-01-01T00:00:00Z\n",
            "2026-01-01T00:00:00Z\n  \n",
            "2026-01-01T00:00:00Z",
        ] {
            assert_eq!(decode_status(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn the_status_file_sits_in_the_status_dir_under_the_worker_id() {
        assert_eq!(
            status_path(Path::new("/s"), "A"),
            PathBuf::from("/s/status/A")
        );
    }
}
