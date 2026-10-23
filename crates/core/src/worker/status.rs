use crate::mcp::string_arg;
use crate::sanitize::strip_controls;
use chrono::DateTime;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const STATUS_DIR: &str = "status";
pub const STATUS_MAX_CHARS: usize = 120;
pub const STATUS_HISTORY: usize = 5;

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

pub fn encode_statuses(history: &[StatusLine]) -> String {
    history
        .iter()
        .map(|status| format!("{}\n{}\n", status.at, status.text))
        .collect()
}

pub fn decode_statuses(raw: &str) -> Vec<StatusLine> {
    let lines: Vec<&str> = raw.lines().collect();
    lines
        .chunks_exact(2)
        .filter_map(|pair| {
            DateTime::parse_from_rfc3339(pair[0]).ok()?;
            let text = pair[1].trim();
            (!text.is_empty()).then(|| StatusLine {
                at: pair[0].to_string(),
                text: text.to_string(),
            })
        })
        .collect()
}

pub fn decode_status(raw: &str) -> Option<StatusLine> {
    decode_statuses(raw).pop()
}

pub fn append_status(mut history: Vec<StatusLine>, status: StatusLine) -> Vec<StatusLine> {
    history.push(status);
    let excess = history.len().saturating_sub(STATUS_HISTORY);
    history.drain(..excess);
    history
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

    fn status(minute: usize, text: &str) -> StatusLine {
        StatusLine {
            at: format!("2026-01-01T00:{minute:02}:00+00:00"),
            text: text.into(),
        }
    }

    #[test]
    fn encode_and_decode_round_trip() {
        let history = [status(0, "Fixing the build."), status(1, "Running tests.")];
        let raw = encode_statuses(&history);
        assert_eq!(
            raw,
            "2026-01-01T00:00:00+00:00\nFixing the build.\n2026-01-01T00:01:00+00:00\nRunning tests.\n"
        );
        assert_eq!(decode_statuses(&raw), history);
        assert_eq!(decode_status(&raw), Some(status(1, "Running tests.")));
    }

    #[test]
    fn an_old_single_entry_file_decodes() {
        let raw = "2026-01-01T00:00:00+00:00\nFixing the build.\n";
        assert_eq!(decode_statuses(raw), vec![status(0, "Fixing the build.")]);
        assert_eq!(decode_status(raw), Some(status(0, "Fixing the build.")));
    }

    #[test]
    fn an_entry_without_a_valid_time_or_text_is_skipped() {
        let raw =
            "not a time\nlost\n2026-01-01T00:01:00+00:00\n  \n2026-01-01T00:02:00+00:00\nkept\n";
        assert_eq!(decode_statuses(raw), vec![status(2, "kept")]);
    }

    #[test]
    fn appending_keeps_the_last_five_oldest_first() {
        let mut history = Vec::new();
        for minute in 0..7 {
            history = append_status(history, status(minute, &format!("step {minute}")));
        }
        let texts: Vec<_> = history.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["step 2", "step 3", "step 4", "step 5", "step 6"]);
    }

    #[test]
    fn appending_below_the_limit_drops_nothing() {
        let four: Vec<_> = (0..4).map(|m| status(m, "x")).collect();
        assert_eq!(append_status(four, status(4, "y")).len(), 5);
        assert_eq!(
            append_status(Vec::new(), status(0, "a")),
            vec![status(0, "a")]
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
            assert!(decode_statuses(raw).is_empty(), "{raw:?}");
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
