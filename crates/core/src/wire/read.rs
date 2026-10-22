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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Calls {
    pub records: Vec<CallRecord>,
    pub torn_tail: bool,
}

fn decode<'a>(label: &str, bytes: &'a [u8]) -> Result<(&'a str, bool), ReadError> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok((text, false)),
        Err(error) if error.error_len().is_none() => {
            let valid = &bytes[..error.valid_up_to()];
            Ok((std::str::from_utf8(valid).unwrap_or_default(), true))
        }
        Err(error) => Err(ReadError::Call {
            label: label.to_string(),
            line: bytes[..error.valid_up_to()]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count()
                + 1,
            message: error.to_string(),
        }),
    }
}

pub fn parse_calls(label: &str, bytes: Option<&[u8]>) -> Result<Calls, ReadError> {
    let (text, cut) = decode(label, bytes.unwrap_or_default())?;
    let mut calls = Calls {
        torn_tail: cut,
        ..Calls::default()
    };
    for (index, raw) in text.split_inclusive('\n').enumerate() {
        let terminated = raw.ends_with('\n');
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        match serde_json::from_str(line) {
            Ok(record) => calls.records.push(record),
            Err(error) if !terminated && error.classify() == serde_json::error::Category::Eof => {
                calls.torn_tail = true;
            }
            Err(error) => {
                return Err(ReadError::Call {
                    label: label.to_string(),
                    line: index + 1,
                    message: error.to_string(),
                })
            }
        }
    }
    Ok(calls)
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

    fn parse(text: &str) -> Result<Calls, ReadError> {
        parse_calls("c.jsonl", Some(text.as_bytes()))
    }

    #[test]
    fn parses_multi_line_calls() {
        let text = format!("{}\n{}\n", call_line(1), call_line(2));
        let calls = parse(&text).unwrap();
        let seqs = calls.records.iter().map(|c| c.seq).collect::<Vec<_>>();
        assert_eq!(seqs, vec![1, 2]);
        assert!(!calls.torn_tail);
    }

    #[test]
    fn bad_line_names_label_and_line() {
        let text = format!("{}\nnot json\n", call_line(1));
        let error = parse(&text).unwrap_err();
        assert!(error.to_string().starts_with("c.jsonl:2: "), "{error}");
    }

    #[test]
    fn blank_line_is_a_bad_line() {
        let text = format!("{}\n\n{}\n", call_line(1), call_line(2));
        let error = parse(&text).unwrap_err();
        assert!(error.to_string().starts_with("c.jsonl:2: "), "{error}");
    }

    #[test]
    fn missing_file_is_empty_and_not_torn() {
        let calls = parse_calls("c.jsonl", None).unwrap();
        assert!(calls.records.is_empty());
        assert!(!calls.torn_tail);
    }

    #[test]
    fn truncated_last_line_is_skipped_and_flagged() {
        let text = format!("{}\n{{\"seq\":2,\"started", call_line(1));
        let calls = parse(&text).unwrap();
        assert_eq!(calls.records.len(), 1);
        assert!(calls.torn_tail);
    }

    #[test]
    fn unterminated_last_line_that_parses_is_kept() {
        let text = format!("{}\n{}", call_line(1), call_line(2));
        let calls = parse(&text).unwrap();
        assert_eq!(calls.records.len(), 2);
        assert!(!calls.torn_tail);
    }

    #[test]
    fn unterminated_last_line_with_syntax_error_is_an_error() {
        for tail in ["{\"seq\":1}x", "not json"] {
            let text = format!("{}\n{tail}", call_line(1));
            let error = parse(&text).unwrap_err();
            assert!(error.to_string().starts_with("c.jsonl:2: "), "{error}");
        }
    }

    #[test]
    fn tail_cut_inside_a_multibyte_char_is_torn() {
        let mut bytes = format!("{}\n{{\"seq\":2,\"p\":\"", call_line(1)).into_bytes();
        bytes.push("é".as_bytes()[0]);
        let calls = parse_calls("c.jsonl", Some(&bytes)).unwrap();
        assert_eq!(calls.records.len(), 1);
        assert!(calls.torn_tail);
    }

    #[test]
    fn cut_right_after_a_newline_is_torn() {
        let mut bytes = format!("{}\n", call_line(1)).into_bytes();
        bytes.push("é".as_bytes()[0]);
        let calls = parse_calls("c.jsonl", Some(&bytes)).unwrap();
        assert_eq!(calls.records.len(), 1);
        assert!(calls.torn_tail);
    }

    #[test]
    fn invalid_utf8_names_its_line() {
        let mut bytes = format!("{}\n", call_line(1)).into_bytes();
        bytes.extend_from_slice(b"ab\xff\n");
        bytes.extend_from_slice(call_line(3).as_bytes());
        bytes.push(b'\n');
        let error = parse_calls("c.jsonl", Some(&bytes)).unwrap_err();
        assert!(error.to_string().starts_with("c.jsonl:2: "), "{error}");
    }

    #[test]
    fn terminated_truncated_line_is_an_error() {
        let text = format!(
            "{}\n{{\"seq\":2,\"started\n{}\n",
            call_line(1),
            call_line(3)
        );
        let error = parse(&text).unwrap_err();
        assert!(error.to_string().starts_with("c.jsonl:2: "), "{error}");
        let text = "{\"seq\":2,\"started\n";
        let error = parse(text).unwrap_err();
        assert!(error.to_string().starts_with("c.jsonl:1: "), "{error}");
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
        let calls = parse(&text).unwrap();
        assert_eq!(find_call(&calls.records, 2, "s").unwrap().seq, 2);
        let error = find_call(&calls.records, 1, "s").unwrap_err();
        assert_eq!(error.to_string(), "no call 1 in session s");
    }

    #[test]
    fn find_call_does_not_find_the_torn_tail_record() {
        let torn = call_line(2);
        let text = format!("{}\n{}", call_line(1), &torn[..torn.len() / 2]);
        let calls = parse(&text).unwrap();
        assert!(calls.torn_tail);
        assert_eq!(find_call(&calls.records, 1, "s").unwrap().seq, 1);
        let error = find_call(&calls.records, 2, "s").unwrap_err();
        assert_eq!(error.to_string(), "no call 2 in session s");
    }

    #[test]
    fn invalid_utf8_mid_stream_is_an_error() {
        let mut bytes = call_line(1).into_bytes();
        bytes.extend_from_slice(b"\n\xff\xfe\n");
        bytes.extend_from_slice(call_line(2).as_bytes());
        let error = parse_calls("c.jsonl", Some(&bytes)).unwrap_err();
        assert!(matches!(error, ReadError::Call { line: 2, .. }), "{error}");
    }

    #[test]
    fn truncated_utf8_at_the_end_is_a_torn_tail() {
        let mut bytes = call_line(1).into_bytes();
        bytes.extend_from_slice(b"\n\xc3");
        let calls = parse_calls("c.jsonl", Some(&bytes)).unwrap();
        assert!(calls.torn_tail);
        assert_eq!(calls.records.len(), 1);
    }
}
