// Fixtures are hand-written from openai-openapi 2.3.0 (local copy of the spec):
// chat-request.json follows CreateChatCompletionRequest, PromptCacheBreakpointParam
// and PromptCacheOptionsParam; chat-json-usage.json follows
// CreateChatCompletionResponse with CompletionUsage; chat-usage.sse and
// chat-no-usage.sse follow CreateChatCompletionStreamResponse (usage only in the
// last chunk when ChatCompletionStreamOptions.include_usage is set);
// chat-error.json follows the error envelope.
use super::*;
use crate::wire::provider::{assert_provider_contract, provider, ContractCase};
use crate::wire::response::view;
use crate::wire::sse::SseDecoder;
use crate::wire::usage::{input_equivalent, UsageOutcome, FIVE_MIN};
use serde_json::json;

const REQUEST: &str = include_str!("../fixtures/openai/chat-request.json");
const JSON_USAGE: &[u8] = include_bytes!("../fixtures/openai/chat-json-usage.json");
const USAGE_SSE: &[u8] = include_bytes!("../fixtures/openai/chat-usage.sse");
const NO_USAGE_SSE: &[u8] = include_bytes!("../fixtures/openai/chat-no-usage.sse");
const ERROR_JSON: &[u8] = include_bytes!("../fixtures/openai/chat-error.json");
const SSE: Option<&str> = Some("text/event-stream");
const JSON: Option<&str> = Some("application/json");

fn request() -> Value {
    serde_json::from_str(REQUEST).unwrap()
}

fn json_usage() -> Usage {
    Usage {
        input_uncached: 500,
        cache_read: 400,
        cache_writes: vec![CacheWrite {
            tier: THIRTY_MIN,
            tokens: 100,
        }],
        output: 80,
        reasoning: Some(30),
        other: vec![("accepted_prediction_tokens".to_string(), 5)],
    }
}

fn sse_usage() -> Usage {
    Usage {
        input_uncached: 214,
        cache_read: 1536,
        cache_writes: vec![CacheWrite {
            tier: THIRTY_MIN,
            tokens: 256,
        }],
        output: 12,
        reasoning: Some(4),
        other: vec![],
    }
}

fn head(method: &str, path: &str) -> RequestHead {
    RequestHead {
        method: method.into(),
        path: path.into(),
        headers: vec![],
    }
}

#[test]
fn contract() {
    let cases = [
        ContractCase {
            name: "json usage",
            view: view(JSON, None, JSON_USAGE).unwrap(),
            expected: UsageOutcome::Reported(json_usage()),
        },
        ContractCase {
            name: "sse usage",
            view: view(SSE, None, USAGE_SSE).unwrap(),
            expected: UsageOutcome::Reported(sse_usage()),
        },
        ContractCase {
            name: "sse without include_usage",
            view: view(SSE, None, NO_USAGE_SSE).unwrap(),
            expected: UsageOutcome::Missing,
        },
        ContractCase {
            name: "error",
            view: view(JSON, None, ERROR_JSON).unwrap(),
            expected: UsageOutcome::Missing,
        },
    ];
    assert_provider_contract(&OpenAi, &cases, &request());
    assert_eq!(provider(ProviderId::OpenAi).id(), ProviderId::OpenAi);
}

#[test]
fn json_usage_keeps_the_raw_object_and_maps_the_subset_convention() {
    let result = OpenAi.usage(&view(JSON, None, JSON_USAGE).unwrap());
    assert_eq!(result.error, None);
    assert_eq!(
        result.raw.as_ref().and_then(|raw| raw.get("total_tokens")),
        Some(&json!(1080))
    );
    assert_eq!(result.outcome, UsageOutcome::Reported(json_usage()));
}

#[test]
fn absent_cache_write_tokens_means_no_writes_not_zero() {
    let absent = json!({"usage": {"prompt_tokens": 10, "completion_tokens": 2}});
    let present = json!({"usage": {
        "prompt_tokens": 10,
        "completion_tokens": 2,
        "prompt_tokens_details": {"cached_tokens": 4, "cache_write_tokens": 0}
    }});
    let UsageOutcome::Reported(without) = OpenAi.usage(&ResponseView::Json(absent)).outcome else {
        panic!("not reported");
    };
    assert!(without.cache_writes.is_empty());
    assert_eq!(without.input_uncached, 10);
    assert_eq!(without.reasoning, None);
    let UsageOutcome::Reported(with) = OpenAi.usage(&ResponseView::Json(present)).outcome else {
        panic!("not reported");
    };
    assert_eq!(
        with.cache_writes,
        vec![CacheWrite {
            tier: THIRTY_MIN,
            tokens: 0
        }]
    );
    assert_eq!(with.input_uncached, 6);
}

#[test]
fn uncached_input_saturates_when_details_exceed_the_prompt() {
    let body = json!({"usage": {
        "prompt_tokens": 10,
        "completion_tokens": 1,
        "prompt_tokens_details": {"cached_tokens": 8, "cache_write_tokens": 8}
    }});
    let UsageOutcome::Reported(usage) = OpenAi.usage(&ResponseView::Json(body)).outcome else {
        panic!("not reported");
    };
    assert_eq!(usage.input_uncached, 0);
}

#[test]
fn json_without_usage_is_missing_and_error_body_carries_its_message() {
    let none = OpenAi.usage(&ResponseView::Json(json!({"object": "chat.completion"})));
    assert_eq!(none.outcome, UsageOutcome::Missing);
    assert_eq!(none.raw, None);
    let error = OpenAi.usage(&view(JSON, None, ERROR_JSON).unwrap());
    assert_eq!(error.outcome, UsageOutcome::Missing);
    assert_eq!(
        error.error.as_deref(),
        Some("Rate limit reached for gpt-6.1-sol")
    );
}

#[test]
fn sse_without_usage_is_missing_and_done_is_ignored() {
    let result = OpenAi.usage(&view(SSE, None, NO_USAGE_SSE).unwrap());
    assert_eq!(result.outcome, UsageOutcome::Missing);
    assert_eq!(result.raw, None);
    assert_eq!(result.error, None);
}

#[test]
fn sse_error_event_is_recorded() {
    let body = b"data: {\"error\":{\"message\":\"overloaded\"}}\n\n";
    let result = OpenAi.usage(&view(SSE, None, body).unwrap());
    assert_eq!(result.outcome, UsageOutcome::Missing);
    assert_eq!(result.error.as_deref(), Some("overloaded"));
}

#[test]
fn error_without_message_falls_back_to_the_error_object() {
    let result = OpenAi.usage(&ResponseView::Json(json!({"error": {"code": "x"}})));
    assert_eq!(result.error.as_deref(), Some("{\"code\":\"x\"}"));
}

#[test]
fn last_non_null_usage_wins() {
    let body = b"data: {\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n\
        data: {\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n\
        data: {\"usage\":null}\n\ndata: [DONE]\n\n";
    let UsageOutcome::Reported(usage) = OpenAi.usage(&view(SSE, None, body).unwrap()).outcome
    else {
        panic!("not reported");
    };
    assert_eq!((usage.input_uncached, usage.output), (7, 3));
}

#[test]
fn awkward_splits_give_identical_usage() {
    let expected = OpenAi.usage(&view(SSE, None, USAGE_SSE).unwrap());
    assert_eq!(expected.outcome, UsageOutcome::Reported(sse_usage()));
    for at in 0..=USAGE_SSE.len() {
        let mut decoder = SseDecoder::default();
        let mut events = decoder.push(&USAGE_SSE[..at]);
        events.extend(decoder.push(&USAGE_SSE[at..]));
        assert_eq!(
            OpenAi.usage(&ResponseView::Events(events)),
            expected,
            "split at {at}"
        );
    }
}

#[test]
fn segments_follow_tools_system_messages_order_with_labels() {
    let segments = OpenAi.segments(&request());
    let labels: Vec<&str> = segments.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "tools[0] get_weather",
            "tools[1] get_time",
            "system[0]",
            "system[1].0 text",
            "msg[2].0 user/text",
            "msg[2].1 user/text",
            "msg[3] assistant/tool_calls",
            "msg[4] tool/text",
            "msg[5] user/text",
        ]
    );
    let sources: Vec<SegmentSource> = segments.iter().map(|s| s.source).collect();
    assert!(sources.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(sources[2], SegmentSource::System);
    assert_eq!(sources[3], SegmentSource::System);
    assert_eq!(sources[4], SegmentSource::Messages);
}

#[test]
fn breakpoint_marks_its_part_and_is_stripped_from_content() {
    let segments = OpenAi.segments(&request());
    let marked = &segments[4];
    assert_eq!(marked.breakpoint, Some(THIRTY_MIN));
    assert!(marked.text.contains("prompt_cache_breakpoint"));
    assert!(!marked.content.contains("prompt_cache_breakpoint"));
    assert!(segments
        .iter()
        .enumerate()
        .all(|(i, s)| (i == 4) == s.breakpoint.is_some()));
}

#[test]
fn breakpoint_ttl_variants() {
    let part =
        |marker: Value| json!({"type": "text", "text": "x", "prompt_cache_breakpoint": marker});
    assert_eq!(
        breakpoint(&part(json!({"mode": "explicit"}))),
        Some(THIRTY_MIN)
    );
    assert_eq!(
        breakpoint(&part(json!({"mode": "explicit", "ttl": "12h"}))),
        Some(CacheTier::Unspecified)
    );
    assert_eq!(breakpoint(&part(Value::Null)), None);
}

#[test]
fn tool_calls_after_a_system_run_are_not_system_and_late_system_is_a_message() {
    let body = json!({"messages": [
        {"role": "user", "content": "a"},
        {"role": "system", "content": "late"},
    ]});
    let segments = OpenAi.segments(&body);
    assert!(segments.iter().all(|s| s.source == SegmentSource::Messages));
    assert_eq!(segments[1].label, "msg[1] system/text");
}

#[test]
fn classify_accepts_only_post_to_chat_completions() {
    let kind = |m, p| OpenAi.classify(&head(m, p));
    assert_eq!(kind("POST", "/v1/chat/completions"), CallKind::Inference);
    assert_eq!(
        kind("post", "/v1/chat/completions?x=1"),
        CallKind::Inference
    );
    assert_eq!(kind("GET", "/v1/chat/completions"), CallKind::Other);
    assert_eq!(kind("POST", "/v1/models"), CallKind::Other);
    assert_eq!(kind("POST", "/v1/chat/completions/x"), CallKind::Other);
}

#[test]
fn prefix_cache_is_implicit_unless_explicit_mode() {
    let implicit = PrefixCache::Implicit { tier: THIRTY_MIN };
    assert_eq!(OpenAi.prefix_cache(&request()), implicit);
    let mode = |m: &str| json!({"prompt_cache_options": {"mode": m, "ttl": "30m"}});
    assert_eq!(OpenAi.prefix_cache(&mode("implicit")), implicit);
    assert_eq!(
        OpenAi.prefix_cache(&mode("explicit")),
        PrefixCache::Explicit
    );
}

#[test]
fn hint_adds_the_v1_suffix() {
    assert_eq!(
        OpenAi.base_url_hint("127.0.0.1:8080"),
        "export OPENAI_BASE_URL=http://127.0.0.1:8080/v1"
    );
}

#[test]
fn facts_pick_the_model_and_listed_params() {
    let facts = OpenAi.request_facts(&request());
    assert_eq!(facts.model.as_deref(), Some("gpt-6.1-sol"));
    let keys: Vec<&str> = facts.params.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "tool_choice",
            "reasoning_effort",
            "prompt_cache_key",
            "stream_options",
            "max_completion_tokens"
        ]
    );
    assert_eq!(OpenAi.request_facts(&json!({})).model, None);
}

#[test]
fn rates_are_ratios_of_the_listed_model_and_ignore_the_date_suffix() {
    for model in ["gpt-6.1-sol", "gpt-6.1-sol-2026-09-01"] {
        let rates = OpenAi.rates(model);
        assert_eq!(rates.cache_read, 0.05);
        assert_eq!(rates.output, 5.0);
        assert_eq!(
            rates.cache_writes,
            vec![TierRate {
                tier: THIRTY_MIN,
                multiplier: 1.25
            }]
        );
    }
    assert_eq!(OpenAi.rates("gpt-6-astra").cache_read, 0.1);
}

#[test]
fn unknown_model_gets_the_default_ratios() {
    let rates = OpenAi.rates("gpt-unlisted");
    assert_eq!(rates.cache_read, DEFAULT_CACHE_READ);
    assert_eq!(rates.output, DEFAULT_OUTPUT);
    assert_eq!(rates.cache_writes[0].multiplier, DEFAULT_CACHE_WRITE);
    assert!(rates.cache_writes.iter().all(|rate| rate.tier != FIVE_MIN));
}

#[test]
fn input_equivalent_prices_the_json_fixture() {
    let value = input_equivalent(&json_usage(), &OpenAi.rates("gpt-6.1-sol")).unwrap();
    assert!((value - (500.0 + 400.0 * 0.05 + 100.0 * 1.25 + 80.0 * 5.0)).abs() < 1e-9);
}

#[test]
fn prices_and_factor_follow_the_moved_table() {
    let table = OpenAi.prices();
    assert_eq!(table.as_of, "2026-10-28");
    assert_eq!(table.entries.len(), 4);
    assert_eq!(OpenAi.price_factor(&json!({}), "gpt-6.1-sol"), Some(1.0));
}

#[test]
fn headers_and_secrets() {
    assert!(OpenAi.secret_headers().is_empty());
    assert_eq!(
        OpenAi.cache_relevant_headers(),
        ["openai-organization", "openai-project"]
    );
    let stored = OpenAi.stored_response_headers();
    assert!(stored.names.contains(&"x-request-id"));
    assert_eq!(stored.prefixes, ["x-ratelimit-"]);
    assert_eq!(OpenAi.default_upstream(), "https://api.openai.com");
}

#[test]
fn ids_serialize_lowercase() {
    assert_eq!(
        serde_json::to_string(&ProviderId::OpenAi).unwrap(),
        "\"openai\""
    );
    assert_eq!(ProviderId::OpenAi.as_str(), "openai");
}
