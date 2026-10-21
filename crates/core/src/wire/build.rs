use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{DateTime, Utc};

use super::headers::{header_value, stored_response_headers};
use super::provider::{Provider, RequestFacts, RequestHead, ResponseUsage, ResponseView};
use super::record::{CallRecord, RequestBody};
use super::redact::redact_headers;
use super::response::view;
use super::sse::SseEvent;
use super::usage::{input_equivalent, UsageOutcome};

pub struct CallInputs {
    pub seq: u64,
    pub started_at: DateTime<Utc>,
    pub first_byte_at: Option<DateTime<Utc>>,
    pub ended_at: DateTime<Utc>,
    pub head: RequestHead,
    pub request_body: Vec<u8>,
    pub status: Option<u16>,
    pub response_headers: Vec<(String, String)>,
    pub response_body: Vec<u8>,
    pub transport_error: Option<String>,
    pub store_events: bool,
}

struct ResponseSummary {
    usage: ResponseUsage,
    events: Option<Vec<SseEvent>>,
}

fn summarize_response(
    provider: &dyn Provider,
    headers: &[(String, String)],
    body: &[u8],
) -> ResponseSummary {
    if body.is_empty() {
        return ResponseSummary {
            usage: ResponseUsage {
                raw: None,
                outcome: UsageOutcome::Missing,
                error: None,
            },
            events: None,
        };
    }
    let parsed = view(
        header_value(headers, "content-type"),
        header_value(headers, "content-encoding"),
        body,
    );
    match parsed {
        Ok(response) => {
            let usage = provider.usage(&response);
            let events = match response {
                ResponseView::Events(events) => Some(events),
                ResponseView::Json(_) => None,
            };
            ResponseSummary { usage, events }
        }
        Err(error) => ResponseSummary {
            usage: ResponseUsage {
                raw: None,
                outcome: UsageOutcome::Missing,
                error: Some(error.to_string()),
            },
            events: None,
        },
    }
}

fn parse_request(provider: &dyn Provider, body: &[u8]) -> (RequestFacts, RequestBody) {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(json) => (provider.request_facts(&json), RequestBody::Json(json)),
        Err(_) => (
            RequestFacts {
                model: None,
                params: serde_json::Map::new(),
            },
            RequestBody::Base64(STANDARD.encode(body)),
        ),
    }
}

fn equivalent(provider: &dyn Provider, model: Option<&str>, outcome: &UsageOutcome) -> Option<f64> {
    let model = model?;
    match outcome {
        UsageOutcome::Reported(usage) | UsageOutcome::Partial(usage) => {
            input_equivalent(usage, &provider.rates(model)).ok()
        }
        UsageOutcome::Missing => None,
    }
}

pub fn build_call(provider: &dyn Provider, inputs: CallInputs) -> CallRecord {
    let kind = provider.classify(&inputs.head);
    let (facts, request_body) = parse_request(provider, &inputs.request_body);
    let summary = summarize_response(provider, &inputs.response_headers, &inputs.response_body);
    let input_equivalent = equivalent(provider, facts.model.as_deref(), &summary.usage.outcome);
    let complete = inputs.transport_error.is_none();
    CallRecord {
        seq: inputs.seq,
        started_at: inputs.started_at,
        first_byte_at: inputs.first_byte_at,
        ended_at: inputs.ended_at,
        method: inputs.head.method,
        path: inputs.head.path,
        status: inputs.status,
        kind,
        facts,
        request_headers: redact_headers(&inputs.head.headers, provider.secret_headers()),
        response_headers: stored_response_headers(&inputs.response_headers),
        request_body,
        raw_usage: summary.usage.raw,
        usage: summary.usage.outcome,
        input_equivalent,
        complete,
        error: inputs.transport_error.or(summary.usage.error),
        response_events: summary.events.filter(|_| inputs.store_events),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::provider::ProviderId;
    use crate::wire::provider::{provider, CallKind};
    use crate::wire::redact::REDACTED;
    use chrono::TimeZone;

    fn h(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    fn inputs(request_body: &[u8], response_body: &[u8]) -> CallInputs {
        let at = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        CallInputs {
            seq: 1,
            started_at: at,
            first_byte_at: Some(at),
            ended_at: at,
            head: RequestHead {
                method: "POST".into(),
                path: "/v1/messages".into(),
                headers: vec![h("x-api-key", "secret"), h("accept", "*/*")],
            },
            request_body: request_body.to_vec(),
            status: Some(200),
            response_headers: vec![
                h("content-type", "text/event-stream"),
                h("anthropic-organization-id", "org"),
                h("anthropic-ratelimit-requests-limit", "50"),
            ],
            response_body: response_body.to_vec(),
            transport_error: None,
            store_events: false,
        }
    }

    const SSE: &[u8] = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":4}}\n\n";

    fn build(inputs: CallInputs) -> CallRecord {
        build_call(provider(ProviderId::Anthropic).as_ref(), inputs)
    }

    #[test]
    fn inference_record() {
        let record = build(inputs(
            br#"{"model":"claude-opus-4-1","max_tokens":5}"#,
            SSE,
        ));
        assert_eq!(record.kind, CallKind::Inference);
        assert_eq!(record.facts.model.as_deref(), Some("claude-opus-4-1"));
        assert_eq!(record.request_headers[0].1, REDACTED);
        assert_eq!(record.request_headers[1].1, "*/*");
        assert_eq!(
            record.response_headers,
            vec![
                h("content-type", "text/event-stream"),
                h("anthropic-ratelimit-requests-limit", "50")
            ]
        );
        assert_eq!(record.input_equivalent, Some(30.0));
        assert!(record.complete);
        assert_eq!(record.error, None);
        assert!(record.response_events.is_none());
    }

    #[test]
    fn events_stored_only_on_request() {
        let mut i = inputs(b"{}", SSE);
        i.store_events = true;
        assert_eq!(build(i).response_events.map(|e| e.len()), Some(2));
    }

    #[test]
    fn non_json_request_is_base64() {
        let record = build(inputs(&[0, 1, 2], SSE));
        assert_eq!(record.request_body, RequestBody::Base64("AAEC".into()));
        assert_eq!(record.facts.model, None);
        assert_eq!(record.input_equivalent, None);
    }

    #[test]
    fn view_error_becomes_error_with_missing_usage() {
        let mut i = inputs(b"{}", b"not json");
        i.response_headers = vec![h("content-type", "application/json")];
        let record = build(i);
        assert_eq!(record.usage, UsageOutcome::Missing);
        assert!(record.error.unwrap().contains("invalid json"));
        assert!(record.complete);
    }

    #[test]
    fn empty_response_body_is_missing_without_error() {
        let record = build(inputs(b"{}", b""));
        assert_eq!(record.usage, UsageOutcome::Missing);
        assert_eq!(record.error, None);
    }

    #[test]
    fn transport_error_marks_incomplete_and_wins() {
        let mut i = inputs(b"{}", b"");
        i.transport_error = Some("client disconnected".into());
        let record = build(i);
        assert!(!record.complete);
        assert_eq!(record.error.as_deref(), Some("client disconnected"));
    }

    #[test]
    fn unrated_tier_has_no_equivalent() {
        let body = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"cache_creation_input_tokens\":5}}}\n\n";
        let record = build(inputs(br#"{"model":"m"}"#, body));
        assert_eq!(record.input_equivalent, None);
    }
}
