use super::*;
use crate::wire::provider::{assert_provider_contract, provider, ContractCase};
use crate::wire::record::{CallRecord, RequestBody};
use crate::wire::redact::redact_headers;
use crate::wire::response::view;
use crate::wire::sse::SseDecoder;
use crate::wire::usage::input_equivalent;
use chrono::{TimeZone, Utc};
use serde_json::json;

const GZIP: &[u8] = include_bytes!("../fixtures/anthropic/haiku-ok-sse.gz");
const JSON_USAGE: &[u8] = include_bytes!("../fixtures/anthropic/json-usage.json");
const DISAGREE: &[u8] = include_bytes!("../fixtures/anthropic/iterations-disagree.sse");
const PARTIAL: &[u8] = include_bytes!("../fixtures/anthropic/partial.sse");
const ERROR: &[u8] = include_bytes!("../fixtures/anthropic/error.sse");
const REQUEST: &str = include_str!("../fixtures/anthropic/request.json");
const SSE: Option<&str> = Some("text/event-stream; charset=utf-8");
const JSON: Option<&str> = Some("application/json");

fn gzip_view() -> ResponseView {
    view(SSE, Some("gzip"), GZIP).unwrap()
}

fn haiku_usage() -> Usage {
    Usage {
        input_uncached: 2,
        cache_read: 16503,
        cache_writes: vec![],
        output: 4,
        reasoning: Some(0),
        other: vec![],
    }
}

fn json_usage_expected() -> Usage {
    Usage {
        input_uncached: 2,
        cache_read: 2840,
        cache_writes: vec![CacheWrite {
            tier: ONE_HOUR,
            tokens: 3975,
        }],
        output: 879,
        reasoning: Some(658),
        other: vec![],
    }
}

#[test]
fn contract() {
    let partial_usage = Usage {
        input_uncached: 5,
        cache_read: 50,
        cache_writes: vec![],
        output: 1,
        reasoning: None,
        other: vec![],
    };
    let disagree = Usage {
        input_uncached: 10,
        cache_read: 100,
        cache_writes: vec![],
        output: 20,
        reasoning: None,
        other: vec![],
    };
    let cases = [
        ContractCase {
            name: "gzip sse",
            view: gzip_view(),
            expected: UsageOutcome::Reported(haiku_usage()),
        },
        ContractCase {
            name: "json usage",
            view: view(JSON, None, JSON_USAGE).unwrap(),
            expected: UsageOutcome::Reported(json_usage_expected()),
        },
        ContractCase {
            name: "partial",
            view: view(SSE, None, PARTIAL).unwrap(),
            expected: UsageOutcome::Partial(partial_usage),
        },
        ContractCase {
            name: "iterations disagree",
            view: view(SSE, None, DISAGREE).unwrap(),
            expected: UsageOutcome::Reported(disagree),
        },
        ContractCase {
            name: "json without usage",
            view: ResponseView::Json(json!({"type": "message"})),
            expected: UsageOutcome::Missing,
        },
        ContractCase {
            name: "events without usage",
            view: ResponseView::Events(vec![]),
            expected: UsageOutcome::Missing,
        },
    ];
    let request: Value = serde_json::from_str(REQUEST).unwrap();
    assert_provider_contract(&Anthropic, &cases, &request);
}

#[test]
fn top_level_wins_over_iterations() {
    let result = Anthropic.usage(&view(SSE, None, DISAGREE).unwrap());
    let raw = result.raw.unwrap();
    assert_eq!(
        raw["iterations"],
        json!([
            {"input_tokens": 7, "output_tokens": 50, "type": "message"},
            {"input_tokens": 7, "output_tokens": 50, "type": "message"}
        ])
    );
    assert_eq!(raw["output_tokens"], 20);
}

#[test]
fn truncated_gzip_is_partial() {
    let truncated = view(SSE, Some("gzip"), &GZIP[..420]).unwrap();
    assert!(matches!(
        Anthropic.usage(&truncated).outcome,
        UsageOutcome::Partial(_)
    ));
}

#[test]
fn stream_error_is_partial_with_message() {
    let result = Anthropic.usage(&view(SSE, None, ERROR).unwrap());
    assert_eq!(result.error.as_deref(), Some("Overloaded"));
    assert!(matches!(result.outcome, UsageOutcome::Partial(_)));
}

#[test]
fn non_json_error_event_sets_error() {
    let events = vec![SseEvent {
        event: Some("error".into()),
        data: "upstream exploded".into(),
    }];
    let result = Anthropic.usage(&ResponseView::Events(events));
    assert_eq!(result.error.as_deref(), Some("upstream exploded"));
}

#[test]
fn json_error_without_message_serializes_error_object() {
    let body = json!({"type": "error", "error": {"type": "api_error"}});
    let result = Anthropic.usage(&ResponseView::Json(body));
    assert_eq!(result.error.as_deref(), Some(r#"{"type":"api_error"}"#));
}

#[test]
fn json_error_message() {
    let body = json!({"type": "error", "error": {"type": "x", "message": "bad"}});
    let result = Anthropic.usage(&ResponseView::Json(body));
    assert_eq!(result.error.as_deref(), Some("bad"));
    assert_eq!(result.outcome, UsageOutcome::Missing);
}

#[test]
fn unspecified_cache_write_without_breakdown() {
    let body = json!({"usage": {"input_tokens": 1, "cache_creation_input_tokens": 7, "output_tokens": 2, "server_tool_use": {"web_search_requests": 3, "a": 1}}});
    let result = Anthropic.usage(&ResponseView::Json(body));
    let UsageOutcome::Reported(usage) = result.outcome else {
        panic!("not reported");
    };
    assert_eq!(
        usage.cache_writes,
        vec![CacheWrite {
            tier: CacheTier::Unspecified,
            tokens: 7
        }]
    );
    assert_eq!(
        usage.other,
        vec![
            ("server_tool_use.a".to_string(), 1),
            ("server_tool_use.web_search_requests".to_string(), 3)
        ]
    );
}

#[test]
fn awkward_splits_give_identical_usage() {
    let decoded = crate::wire::decode::decode_body(Some("gzip"), GZIP).unwrap();
    let expected = Anthropic.usage(&gzip_view());
    for at in 0..=decoded.len() {
        let mut decoder = SseDecoder::default();
        let mut events = decoder.push(&decoded[..at]);
        events.extend(decoder.push(&decoded[at..]));
        assert_eq!(
            Anthropic.usage(&ResponseView::Events(events)),
            expected,
            "split at {at}"
        );
    }
}

#[test]
fn pipeline_builds_redacted_record() {
    let response = gzip_view();
    let result = Anthropic.usage(&response);
    let UsageOutcome::Reported(usage) = result.outcome.clone() else {
        panic!("not reported");
    };
    let equivalent = input_equivalent(&usage, &Anthropic.rates("claude-haiku-5-5")).unwrap();
    assert!((equivalent - 1672.3).abs() < 1e-6);
    let body: Value = serde_json::from_str(REQUEST).unwrap();
    let ResponseView::Events(events) = response else {
        panic!("not events");
    };
    let at = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let record = CallRecord {
        seq: 1,
        started_at: at,
        first_byte_at: Some(at),
        ended_at: at,
        method: "POST".into(),
        path: "/v1/messages".into(),
        status: Some(200),
        kind: CallKind::Inference,
        facts: Anthropic.request_facts(&body),
        request_headers: redact_headers(
            &[("authorization".into(), "Bearer fixture".into())],
            Anthropic.secret_headers(),
        ),
        response_headers: vec![
            (
                "content-type".into(),
                "text/event-stream; charset=utf-8".into(),
            ),
            ("content-encoding".into(), "gzip".into()),
        ],
        request_body: RequestBody::Json(body),
        raw_usage: result.raw,
        usage: result.outcome,
        input_equivalent: Some(equivalent),
        complete: true,
        error: result.error,
        response_events: Some(events),
    };
    let text = serde_json::to_string(&record).unwrap();
    assert!(!text.contains("Bearer"));
    assert_eq!(serde_json::from_str::<CallRecord>(&text).unwrap(), record);
}

fn seg(
    source: SegmentSource,
    label: &str,
    text: &str,
    breakpoint: Option<CacheTier>,
) -> PromptSegment {
    PromptSegment {
        label: label.into(),
        source,
        text: text.into(),
        content: text.into(),
        breakpoint,
    }
}

#[test]
fn segments_follow_cache_order_with_breakpoints() {
    let body: Value = serde_json::from_str(REQUEST).unwrap();
    let segments = Anthropic.segments(&body);
    let labels: Vec<&str> = segments.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "tools[0] Read",
            "system[0]",
            "system[1]",
            "msg[0].0 user/text",
            "msg[1].0 assistant/text"
        ]
    );
    let tiers: Vec<Option<CacheTier>> = segments.iter().map(|s| s.breakpoint).collect();
    assert_eq!(
        tiers,
        [Some(ONE_HOUR), None, Some(FIVE_MIN), None, Some(FIVE_MIN)]
    );
    assert_eq!(segments[1].text, body["system"][0].to_string());
}

#[test]
fn content_omits_the_cache_control_marker_but_text_keeps_it() {
    let bare = json!({"type": "text", "text": "a"});
    let marked = json!({"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}});
    let body = json!({"system": [marked.clone()]});
    let segment = &Anthropic.segments(&body)[0];
    assert!(segment.text.contains("cache_control"));
    assert!(!segment.content.contains("cache_control"));
    assert_eq!(segment.content, bare.to_string());
    assert_eq!(segment.text, marked.to_string());
}

#[test]
fn explicit_five_minute_ttl_is_five_minutes() {
    let body = json!({"system": [{"type": "text", "text": "a", "cache_control": {"type": "ephemeral", "ttl": "5m"}}]});
    assert_eq!(Anthropic.segments(&body)[0].breakpoint, Some(FIVE_MIN));
}

#[test]
fn segments_carry_their_source() {
    let body: Value = serde_json::from_str(REQUEST).unwrap();
    let sources: Vec<SegmentSource> = Anthropic.segments(&body).iter().map(|s| s.source).collect();
    assert_eq!(
        sources,
        [
            SegmentSource::Tools,
            SegmentSource::System,
            SegmentSource::System,
            SegmentSource::Messages,
            SegmentSource::Messages
        ]
    );
}

fn block_breakpoint(control: Value) -> Option<CacheTier> {
    let body = json!({"system": [{"type": "text", "text": "a", "cache_control": control}]});
    Anthropic.segments(&body)[0].breakpoint
}

#[test]
fn null_cache_control_is_not_a_breakpoint() {
    assert_eq!(block_breakpoint(Value::Null), None);
}

#[test]
fn unknown_ttl_is_unspecified() {
    for ttl in [json!("2h"), json!("1H"), json!(3600)] {
        assert_eq!(
            block_breakpoint(json!({"type": "ephemeral", "ttl": ttl})),
            Some(CacheTier::Unspecified),
            "{ttl}"
        );
    }
}

#[test]
fn missing_ttl_is_five_minutes() {
    assert_eq!(
        block_breakpoint(json!({"type": "ephemeral"})),
        Some(FIVE_MIN)
    );
}

#[test]
fn non_object_json_body_gives_no_segments_or_facts() {
    for body in [json!([]), json!("x")] {
        assert!(Anthropic.segments(&body).is_empty());
        assert_eq!(Anthropic.request_facts(&body).model, None);
    }
}

#[test]
fn string_system_and_content_are_single_segments() {
    let body = json!({"system": "sys", "messages": [{"role": "user", "content": "hi"}]});
    assert_eq!(
        Anthropic.segments(&body),
        [
            seg(
                SegmentSource::System,
                "system[0]",
                r#"{"type":"text","text":"sys"}"#,
                None
            ),
            seg(
                SegmentSource::Messages,
                "msg[0].0 user/text",
                r#"{"type":"text","text":"hi"}"#,
                None
            )
        ]
    );
}

#[test]
fn missing_fields_give_no_segments() {
    assert!(Anthropic.segments(&json!({})).is_empty());
    let body = json!({"messages": [{"content": [{}]}, {"role": "user"}]});
    assert_eq!(
        Anthropic.segments(&body),
        [seg(SegmentSource::Messages, "msg[0].0 ?/?", "{}", None)]
    );
}

#[test]
fn classify_cases() {
    let head = |method: &str, path: &str| RequestHead {
        method: method.into(),
        path: path.into(),
        headers: vec![],
    };
    let cases = [
        ("POST", "/v1/messages", CallKind::Inference),
        ("post", "/v1/messages?beta=true", CallKind::Inference),
        ("POST", "/v1/messages/count_tokens", CallKind::Other),
        ("GET", "/v1/models", CallKind::Other),
        ("GET", "/v1/messages", CallKind::Other),
    ];
    for (method, path, kind) in cases {
        assert_eq!(
            Anthropic.classify(&head(method, path)),
            kind,
            "{method} {path}"
        );
    }
}

#[test]
fn request_facts_from_fixture() {
    let body: Value = serde_json::from_str(REQUEST).unwrap();
    let facts = Anthropic.request_facts(&body);
    assert_eq!(facts.model.as_deref(), Some("claude-haiku-5-5"));
    let keys: Vec<&str> = facts.params.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "thinking",
            "max_tokens",
            "output_config",
            "context_management",
            "metadata"
        ]
    );
}

#[test]
fn cache_read_rates() {
    let cases = [
        ("claude-fable-5-1", 0.025),
        ("claude-mythos-5-1", 0.025),
        ("claude-opus-5-5", 0.05),
        ("claude-opus-5-5[1m]", 0.05),
        ("claude-sonnet-5-5", 0.05),
        ("claude-opus-5", 0.1),
        ("claude-haiku-5-5", 0.1),
        ("claude-opus-5-50", 0.1),
    ];
    for (model, expected) in cases {
        let rates = Anthropic.rates(model);
        assert_eq!(rates.cache_read, expected, "{model}");
        assert_eq!(rates.output, 5.0);
        assert_eq!(rates.cache_writes.len(), 2);
    }
}

#[test]
fn registry_returns_anthropic() {
    assert_eq!(provider(ProviderId::Anthropic).id(), ProviderId::Anthropic);
    assert_eq!(
        provider(ProviderId::Anthropic).default_upstream(),
        "https://api.anthropic.com"
    );
}

#[test]
fn price_table_agrees_with_billing_multipliers() {
    for entry in Anthropic.prices().entries {
        let rates = Anthropic.rates(&entry.model);
        let price = entry.price;
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(
            close(price.cache_read, price.input * rates.cache_read),
            "{}",
            entry.model
        );
        assert!(
            close(price.output, price.input * rates.output),
            "{}",
            entry.model
        );
        for (tier, rate) in price.cache_writes.iter().zip(&rates.cache_writes) {
            assert!(
                close(tier.usd_per_mtok, price.input * rate.multiplier),
                "{}",
                entry.model
            );
        }
    }
}

#[test]
fn price_factor_covers_speed_and_geo() {
    let facts = RequestFacts {
        model: None,
        params: Map::new(),
    };
    let factor = |raw: Value, model: &str| Anthropic.price_factor(&raw, &facts, model);
    assert_eq!(factor(json!({}), "claude-haiku-5-5"), Some(1.0));
    assert_eq!(
        factor(
            json!({"speed": "standard", "inference_geo": "global"}),
            "claude-haiku-5-5"
        ),
        Some(1.0)
    );
    assert_eq!(
        factor(json!({"speed": "fast"}), "claude-opus-5-5"),
        Some(2.0)
    );
    assert_eq!(factor(json!({"speed": "fast"}), "claude-haiku-5-5"), None);
    assert_eq!(
        factor(json!({"inference_geo": "not_available"}), "claude-opus-4-5"),
        Some(1.0)
    );
    assert_eq!(
        factor(json!({"inference_geo": "us"}), "claude-sonnet-5-5[1m]"),
        Some(1.1)
    );
    assert_eq!(
        factor(json!({"inference_geo": "us"}), "claude-opus-4-5-20251101"),
        None
    );
    let both = factor(
        json!({"speed": "fast", "inference_geo": "us"}),
        "claude-opus-5",
    );
    assert!((both.unwrap() - 2.2).abs() < 1e-9);
    assert_eq!(factor(json!({"speed": "turbo"}), "claude-opus-5"), None);
}

#[test]
fn batch_paths_are_not_inference() {
    let head = |path: &str| RequestHead {
        method: "POST".into(),
        path: path.into(),
        headers: vec![],
    };
    assert_eq!(
        Anthropic.classify(&head("/v1/messages/batches")),
        CallKind::Other
    );
    assert_eq!(
        Anthropic.classify(&head("/v1/messages")),
        CallKind::Inference
    );
}

#[test]
fn zero_token_cache_write_is_omitted() {
    let zero = json!({"input_tokens": 1, "cache_creation_input_tokens": 0});
    assert!(cache_writes(&zero).is_empty());
}

#[test]
fn provider_constants_are_anthropic_specific() {
    assert_eq!(
        Anthropic.base_url_hint("127.0.0.1:9"),
        "export ANTHROPIC_BASE_URL=http://127.0.0.1:9"
    );
    assert!(Anthropic.secret_headers().is_empty());
}
