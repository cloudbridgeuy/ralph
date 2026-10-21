use std::fmt;

use super::record::{CallRecord, SessionRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    Session {
        label: String,
        message: String,
    },
    Call {
        label: String,
        line: usize,
        message: String,
    },
    NoSessions {
        root: String,
    },
    UnknownSession {
        id: String,
        root: String,
    },
    UnknownCall {
        seq: u64,
        session: String,
    },
    NoBase {
        seq: u64,
        session: String,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session { label, message } => write!(f, "{label}: {message}"),
            Self::Call {
                label,
                line,
                message,
            } => write!(f, "{label}:{line}: {message}"),
            Self::NoSessions { root } => write!(f, "no sessions in {root}"),
            Self::UnknownSession { id, root } => write!(f, "no session {id} in {root}"),
            Self::UnknownCall { seq, session } => write!(f, "no call {seq} in session {session}"),
            Self::NoBase { seq, session } => write!(
                f,
                "no earlier call in session {session} shares a prompt prefix with call {seq}"
            ),
        }
    }
}

impl std::error::Error for ReadError {}

pub fn parse_session(label: &str, text: &str) -> Result<SessionRecord, ReadError> {
    serde_json::from_str(text).map_err(|error| ReadError::Session {
        label: label.to_string(),
        message: error.to_string(),
    })
}

pub fn parse_calls(label: &str, text: &str) -> Result<Vec<CallRecord>, ReadError> {
    text.lines()
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|error| ReadError::Call {
                label: label.to_string(),
                line: index + 1,
                message: error.to_string(),
            })
        })
        .collect()
}

pub fn pick_session(
    ids: Vec<String>,
    requested: Option<String>,
    root: &str,
) -> Result<String, ReadError> {
    match requested {
        Some(id) if ids.contains(&id) => Ok(id),
        Some(id) => Err(ReadError::UnknownSession {
            id,
            root: root.to_string(),
        }),
        None => ids.into_iter().max().ok_or_else(|| ReadError::NoSessions {
            root: root.to_string(),
        }),
    }
}

pub fn find_call<'a>(
    calls: &'a [CallRecord],
    seq: u64,
    session: &str,
) -> Result<&'a CallRecord, ReadError> {
    calls
        .iter()
        .find(|call| call.seq == seq)
        .ok_or_else(|| ReadError::UnknownCall {
            seq,
            session: session.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::provider::ProviderId;
    use crate::wire::record::SCHEMA_VERSION;
    use crate::wire::usage::UsageOutcome;
    use chrono::{TimeZone, Utc};

    fn call_line(seq: u64) -> String {
        let at = Utc.with_ymd_and_hms(2026, 10, 21, 16, 24, 29).unwrap();
        let record = CallRecord {
            seq,
            started_at: at,
            first_byte_at: None,
            ended_at: at,
            method: "POST".into(),
            path: "/v1/messages".into(),
            status: Some(200),
            kind: crate::wire::provider::CallKind::Inference,
            facts: crate::wire::provider::RequestFacts {
                model: None,
                params: serde_json::Map::new(),
            },
            request_headers: vec![],
            response_headers: vec![],
            request_body: crate::wire::record::RequestBody::Base64(String::new()),
            raw_usage: None,
            usage: UsageOutcome::Missing,
            input_equivalent: None,
            complete: true,
            error: None,
            response_events: None,
        };
        serde_json::to_string(&record).unwrap()
    }

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn parses_multi_line_calls() {
        let text = format!("{}\n{}\n", call_line(1), call_line(2));
        let calls = parse_calls("c.jsonl", &text).unwrap();
        assert_eq!(calls.iter().map(|c| c.seq).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn bad_line_names_label_and_line() {
        let text = format!("{}\nnot json\n", call_line(1));
        let error = parse_calls("c.jsonl", &text).unwrap_err();
        assert!(error.to_string().starts_with("c.jsonl:2: "), "{error}");
    }

    #[test]
    fn blank_line_is_a_bad_line() {
        let text = format!("{}\n\n{}\n", call_line(1), call_line(2));
        let error = parse_calls("c.jsonl", &text).unwrap_err();
        assert!(error.to_string().starts_with("c.jsonl:2: "), "{error}");
    }

    #[test]
    fn parses_session_and_labels_errors() {
        let session = SessionRecord {
            schema_version: SCHEMA_VERSION,
            ralph_version: "0".into(),
            provider: ProviderId::Anthropic,
            upstream: "http://u".into(),
            listen: "127.0.0.1:1".into(),
            started_at: Utc.with_ymd_and_hms(2026, 10, 21, 16, 24, 29).unwrap(),
        };
        let text = serde_json::to_string(&session).unwrap();
        assert_eq!(parse_session("s.json", &text).unwrap(), session);
        let error = parse_session("s.json", "{").unwrap_err();
        assert!(error.to_string().starts_with("s.json: "), "{error}");
    }

    #[test]
    fn latest_is_max_id() {
        assert_eq!(
            pick_session(ids(&["a", "c", "b"]), None, "/r").unwrap(),
            "c"
        );
    }

    #[test]
    fn empty_list_has_no_sessions() {
        let error = pick_session(vec![], None, "/r").unwrap_err();
        assert_eq!(error.to_string(), "no sessions in /r");
    }

    #[test]
    fn unknown_session_is_an_error() {
        let error = pick_session(ids(&["a"]), Some("x".into()), "/r").unwrap_err();
        assert_eq!(error.to_string(), "no session x in /r");
    }

    #[test]
    fn requested_session_is_picked() {
        let id = pick_session(ids(&["a", "b"]), Some("a".into()), "/r").unwrap();
        assert_eq!(id, "a");
    }

    #[test]
    fn find_call_matches_seq_not_position() {
        let text = format!("{}\n{}\n", call_line(5), call_line(2));
        let calls = parse_calls("c.jsonl", &text).unwrap();
        assert_eq!(find_call(&calls, 2, "s").unwrap().seq, 2);
        let error = find_call(&calls, 1, "s").unwrap_err();
        assert_eq!(error.to_string(), "no call 1 in session s");
    }
}
