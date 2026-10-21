use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::provider::{CallKind, ProviderId, RequestFacts};
use super::sse::SseEvent;
use super::usage::UsageOutcome;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub schema_version: u32,
    pub ralph_version: String,
    pub provider: ProviderId,
    pub upstream: String,
    pub listen: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", content = "body", rename_all = "snake_case")]
pub enum RequestBody {
    Json(serde_json::Value),
    Base64(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallRecord {
    pub seq: u64,
    pub started_at: DateTime<Utc>,
    pub first_byte_at: Option<DateTime<Utc>>,
    pub ended_at: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub status: Option<u16>,
    pub kind: CallKind,
    pub facts: RequestFacts,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub request_body: RequestBody,
    pub raw_usage: Option<serde_json::Value>,
    pub usage: UsageOutcome,
    pub input_equivalent: Option<f64>,
    pub complete: bool,
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub response_events: Option<Vec<SseEvent>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::usage::{CacheTier, CacheWrite, Usage};
    use chrono::TimeZone;
    use serde_json::json;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn usage() -> Usage {
        Usage {
            input_uncached: 3,
            cache_read: 10,
            cache_writes: vec![CacheWrite {
                tier: CacheTier::Ttl { seconds: 3600 },
                tokens: 50,
            }],
            output: 7,
            reasoning: Some(2),
            other: vec![("x".into(), 1)],
        }
    }

    fn base() -> CallRecord {
        let mut params = serde_json::Map::new();
        params.insert("max_tokens".into(), json!(10));
        CallRecord {
            seq: 1,
            started_at: t(0),
            first_byte_at: Some(t(1)),
            ended_at: t(2),
            method: "POST".into(),
            path: "/v1/messages".into(),
            status: Some(200),
            kind: CallKind::Inference,
            facts: RequestFacts {
                model: Some("m".into()),
                params,
            },
            request_headers: vec![("a".into(), "b".into())],
            response_headers: vec![],
            request_body: RequestBody::Json(json!({"k": 1})),
            raw_usage: Some(json!({"input_tokens": 3})),
            usage: UsageOutcome::Reported(usage()),
            input_equivalent: Some(1.5),
            complete: true,
            error: None,
            response_events: Some(vec![SseEvent {
                event: Some("message_stop".into()),
                data: "{}".into(),
            }]),
        }
    }

    fn round_trip(record: &CallRecord) {
        let json = serde_json::to_string(record).unwrap();
        assert_eq!(&serde_json::from_str::<CallRecord>(&json).unwrap(), record);
    }

    #[test]
    fn session_round_trip() {
        let s = SessionRecord {
            schema_version: SCHEMA_VERSION,
            ralph_version: "0.1.0".into(),
            provider: ProviderId::Anthropic,
            upstream: "https://api.anthropic.com".into(),
            listen: "127.0.0.1:1".into(),
            started_at: t(0),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<SessionRecord>(&json).unwrap(), s);
    }

    #[test]
    fn streaming_record_round_trip() {
        round_trip(&base());
    }

    #[test]
    fn base64_missing_error_round_trip() {
        let mut r = base();
        r.request_body = RequestBody::Base64("AAEC".into());
        r.usage = UsageOutcome::Missing;
        r.status = None;
        r.error = Some("boom".into());
        r.complete = false;
        r.response_events = None;
        round_trip(&r);
    }

    #[test]
    fn partial_round_trip_and_no_events_key() {
        let mut r = base();
        r.usage = UsageOutcome::Partial(usage());
        r.response_events = None;
        round_trip(&r);
        let value = serde_json::to_value(&r).unwrap();
        assert!(value.get("response_events").is_none());
    }
}
