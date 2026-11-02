// Fixtures are hand-written from openai-openapi 2.3.0 (local copy of the spec):
// chat-request.json follows CreateChatCompletionRequest, PromptCacheBreakpointParam
// and PromptCacheOptionsParam; chat-json-usage.json follows
// CreateChatCompletionResponse with CompletionUsage; chat-usage.sse and
// chat-no-usage.sse follow CreateChatCompletionStreamResponse (usage only in the
// last chunk when ChatCompletionStreamOptions.include_usage is set);
// chat-error.json follows the error envelope. responses-request.json follows
// CreateResponse; responses-json-usage.json follows Response with ResponseUsage;
// the responses-*.sse files follow the response.created, response.in_progress,
// response.output_text.delta, response.completed, response.incomplete and
// response.failed stream events.
use super::*;
use crate::wire::provider::{assert_provider_contract, provider, ContractCase};
use crate::wire::response::view;
use crate::wire::sse::SseDecoder;
use crate::wire::usage::{input_equivalent, UsageOutcome, FIVE_MIN};
use serde_json::{json, Map, Value};

const REQUEST: &str = include_str!("../fixtures/openai/chat-request.json");
const JSON_USAGE: &[u8] = include_bytes!("../fixtures/openai/chat-json-usage.json");
const USAGE_SSE: &[u8] = include_bytes!("../fixtures/openai/chat-usage.sse");
const NO_USAGE_SSE: &[u8] = include_bytes!("../fixtures/openai/chat-no-usage.sse");
const ERROR_JSON: &[u8] = include_bytes!("../fixtures/openai/chat-error.json");
const RESP_REQUEST: &str = include_str!("../fixtures/openai/responses-request.json");
const RESP_JSON: &[u8] = include_bytes!("../fixtures/openai/responses-json-usage.json");
const RESP_COMPLETED: &[u8] = include_bytes!("../fixtures/openai/responses-completed.sse");
const RESP_INCOMPLETE: &[u8] = include_bytes!("../fixtures/openai/responses-incomplete.sse");
const RESP_FAILED: &[u8] = include_bytes!("../fixtures/openai/responses-failed.sse");
const RESP_CUT: &[u8] = include_bytes!("../fixtures/openai/responses-cut.sse");
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

fn responses_usage(details: Value) -> Usage {
    let body = json!({"object": "response", "status": "completed", "usage": {
        "input_tokens": 12000,
        "output_tokens": 30,
        "input_tokens_details": details
    }});
    let UsageOutcome::Reported(usage) = OpenAi.usage(&ResponseView::Json(body)).outcome else {
        panic!("not reported");
    };
    usage
}

#[test]
fn responses_split_cache_writes_by_ttl() {
    let usage = responses_usage(json!({
        "cached_tokens": 4000,
        "cache_write_tokens": 7000,
        "cache_write_tokens_details": {"ttl_30m_tokens": 3000, "ttl_12h_tokens": 4000}
    }));
    assert_eq!(usage.input_uncached, 1000);
    assert_eq!(usage.cache_read, 4000);
    assert_eq!(
        usage.cache_writes,
        vec![
            CacheWrite {
                tier: THIRTY_MIN,
                tokens: 3000
            },
            CacheWrite {
                tier: TWELVE_HOURS,
                tokens: 4000
            }
        ]
    );
}

#[test]
fn responses_keep_a_zero_ttl_entry_and_fall_back_to_one_30m_write() {
    let zero = responses_usage(json!({
        "cached_tokens": 0,
        "cache_write_tokens": 500,
        "cache_write_tokens_details": {"ttl_30m_tokens": 500, "ttl_12h_tokens": 0}
    }));
    assert_eq!(
        zero.cache_writes,
        vec![
            CacheWrite {
                tier: THIRTY_MIN,
                tokens: 500
            },
            CacheWrite {
                tier: TWELVE_HOURS,
                tokens: 0
            }
        ]
    );
    let empty = responses_usage(json!({
        "cached_tokens": 0,
        "cache_write_tokens": 500,
        "cache_write_tokens_details": {}
    }));
    assert_eq!(
        empty.cache_writes,
        vec![CacheWrite {
            tier: THIRTY_MIN,
            tokens: 500
        }]
    );
    let unsplit = responses_usage(json!({"cached_tokens": 0, "cache_write_tokens": 500}));
    assert_eq!(
        unsplit.cache_writes,
        vec![CacheWrite {
            tier: THIRTY_MIN,
            tokens: 500
        }]
    );
    assert_eq!(unsplit.input_uncached, 11500);
}

#[test]
fn openai_rates_and_prices_include_the_12h_write_tier() {
    let rates = OpenAi.rates("gpt-6.1-sol");
    assert!(rates.cache_writes.contains(&TierRate {
        tier: TWELVE_HOURS,
        multiplier: 2.5
    }));
    assert!(OpenAi.rates("gpt-5.2").cache_writes.contains(&TierRate {
        tier: TWELVE_HOURS,
        multiplier: 2.5
    }));
    for entry in OpenAi.prices().entries {
        let twelve = entry
            .price
            .cache_writes
            .iter()
            .find(|write| write.tier == TWELVE_HOURS)
            .unwrap();
        assert_eq!(twelve.usd_per_mtok, entry.price.input * 2.5);
    }
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
        Some(TWELVE_HOURS)
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
    let implicit = PrefixCache::Implicit {
        tier: THIRTY_MIN,
        min_tokens: Some(1_024),
    };
    assert_eq!(OpenAi.prefix_cache(&request()), implicit);
    let mode = |m: &str| json!({"model": "gpt-6.1-sol", "prompt_cache_options": {"mode": m, "ttl": "30m"}});
    assert_eq!(OpenAi.prefix_cache(&mode("implicit")), implicit);
    assert_eq!(
        OpenAi.prefix_cache(&mode("explicit")),
        PrefixCache::Explicit
    );
}

#[test]
fn implicit_tier_follows_the_ttl_or_retention_and_the_listing() {
    let tier = |body: Value| OpenAi.prefix_cache(&body);
    let implicit = |tier| PrefixCache::Implicit {
        tier,
        min_tokens: Some(1_024),
    };
    let unlisted = |tier| PrefixCache::Implicit {
        tier,
        min_tokens: None,
    };
    let listed = json!({"model": "gpt-6.1-sol"});
    assert_eq!(tier(listed.clone()), implicit(THIRTY_MIN));
    assert_eq!(
        tier(json!({"model": "gpt-6.1-sol-2026-09-01"})),
        implicit(THIRTY_MIN)
    );
    assert_eq!(
        tier(json!({"model": "gpt-6.1-sol", "prompt_cache_options": {"ttl": "30m"}})),
        implicit(THIRTY_MIN)
    );
    assert_eq!(
        tier(json!({"model": "gpt-6.1-sol", "prompt_cache_options": {"ttl": "12h"}})),
        implicit(TWELVE_HOURS)
    );
    assert_eq!(
        tier(json!({"model": "gpt-6.1-sol", "prompt_cache_options": {"ttl": "1h"}})),
        implicit(CacheTier::Unspecified)
    );
    assert_eq!(
        tier(json!({"model": "gpt-6.1-sol", "prompt_cache_retention": "24h"})),
        implicit(CacheTier::Unspecified)
    );
    assert_eq!(
        tier(json!({"model": "gpt-6.1-sol", "prompt_cache_retention": "in_memory"})),
        implicit(CacheTier::Unspecified)
    );
    assert_eq!(
        tier(json!({"model": "gpt-5.2"})),
        unlisted(CacheTier::Unspecified)
    );
    assert_eq!(
        tier(json!({"model": "gpt-5.2", "prompt_cache_options": {"ttl": "12h"}})),
        unlisted(TWELVE_HOURS)
    );
    assert_eq!(tier(json!({})), unlisted(CacheTier::Unspecified));
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
            vec![
                TierRate {
                    tier: THIRTY_MIN,
                    multiplier: 1.25
                },
                TierRate {
                    tier: TWELVE_HOURS,
                    multiplier: 2.5
                }
            ]
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
    let facts = tier_facts(None);
    assert_eq!(
        OpenAi.price_factor(&json!({}), &facts, "gpt-6.1-sol"),
        Some(1.0)
    );
}

fn tier_facts(tier: Option<Value>) -> RequestFacts {
    RequestFacts {
        model: Some("gpt-6.1-sol".to_string()),
        params: tier
            .map(|value| Map::from_iter([("service_tier".to_string(), value)]))
            .unwrap_or_default(),
    }
}

#[test]
fn price_factor_prices_only_standard_service_tiers() {
    let factor =
        |tier: Option<Value>| OpenAi.price_factor(&json!({}), &tier_facts(tier), "gpt-6.1-sol");
    assert_eq!(factor(None), Some(1.0));
    assert_eq!(factor(Some(json!("default"))), Some(1.0));
    assert_eq!(factor(Some(json!("auto"))), Some(1.0));
    assert_eq!(factor(Some(Value::Null)), Some(1.0));
    assert_eq!(factor(Some(json!("priority"))), None);
    assert_eq!(factor(Some(json!("flex"))), None);
    assert_eq!(factor(Some(json!("scale"))), None);
    assert_eq!(factor(Some(json!(3))), None);
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

fn resp_request() -> Value {
    serde_json::from_str(RESP_REQUEST).unwrap()
}

fn resp_json_usage() -> Usage {
    Usage {
        input_uncached: 500,
        cache_read: 400,
        cache_writes: vec![CacheWrite {
            tier: THIRTY_MIN,
            tokens: 100,
        }],
        output: 80,
        reasoning: Some(30),
        other: vec![],
    }
}

fn resp_sse_usage() -> Usage {
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

fn resp_usage(body: &[u8]) -> ResponseUsage {
    OpenAi.usage(&view(SSE, None, body).unwrap())
}

#[test]
fn responses_contract() {
    let cases = [
        ContractCase {
            name: "json usage",
            view: view(JSON, None, RESP_JSON).unwrap(),
            expected: UsageOutcome::Reported(resp_json_usage()),
        },
        ContractCase {
            name: "completed sse",
            view: view(SSE, None, RESP_COMPLETED).unwrap(),
            expected: UsageOutcome::Reported(resp_sse_usage()),
        },
        ContractCase {
            name: "failed sse",
            view: view(SSE, None, RESP_FAILED).unwrap(),
            expected: UsageOutcome::Missing,
        },
        ContractCase {
            name: "cut sse",
            view: view(SSE, None, RESP_CUT).unwrap(),
            expected: UsageOutcome::Missing,
        },
    ];
    assert_provider_contract(&OpenAi, &cases, &resp_request());
}

#[test]
fn responses_json_usage_uses_the_subset_convention() {
    let result = OpenAi.usage(&view(JSON, None, RESP_JSON).unwrap());
    assert_eq!(result.error, None);
    assert_eq!(
        result.raw.as_ref().and_then(|raw| raw.get("total_tokens")),
        Some(&json!(1080))
    );
    assert_eq!(result.outcome, UsageOutcome::Reported(resp_json_usage()));
}

#[test]
fn responses_json_statuses() {
    let incomplete = json!({"object": "response", "status": "incomplete",
        "incomplete_details": {"reason": "max_output_tokens"},
        "usage": {"input_tokens": 3, "output_tokens": 4}});
    let result = OpenAi.usage(&ResponseView::Json(incomplete));
    assert_eq!(
        result.error.as_deref(),
        Some("incomplete: max_output_tokens")
    );
    assert!(matches!(result.outcome, UsageOutcome::Reported(_)));
    let failed = json!({"object": "response", "status": "failed",
        "error": {"code": "server_error", "message": "boom"}, "usage": null});
    let result = OpenAi.usage(&ResponseView::Json(failed));
    assert_eq!(result.error.as_deref(), Some("boom"));
    assert_eq!(result.outcome, UsageOutcome::Missing);
    let bare = json!({"object": "response", "status": "incomplete"});
    assert_eq!(
        OpenAi.usage(&ResponseView::Json(bare)).error.as_deref(),
        Some("incomplete: unknown")
    );
    let no_usage = json!({"object": "response", "status": "completed"});
    let result = OpenAi.usage(&ResponseView::Json(no_usage));
    assert_eq!(
        (result.outcome, result.error),
        (UsageOutcome::Missing, None)
    );
}

#[test]
fn responses_stream_terminal_events() {
    let completed = resp_usage(RESP_COMPLETED);
    assert_eq!(completed.outcome, UsageOutcome::Reported(resp_sse_usage()));
    assert_eq!(completed.error, None);
    let incomplete = resp_usage(RESP_INCOMPLETE);
    assert_eq!(
        incomplete.error.as_deref(),
        Some("incomplete: max_output_tokens")
    );
    let UsageOutcome::Reported(usage) = incomplete.outcome else {
        panic!("not reported");
    };
    assert_eq!(
        (usage.input_uncached, usage.output, usage.reasoning),
        (300, 512, Some(500))
    );
    let failed = resp_usage(RESP_FAILED);
    assert_eq!(failed.outcome, UsageOutcome::Missing);
    assert_eq!(failed.error.as_deref(), Some("The model failed to respond"));
    let cut = resp_usage(RESP_CUT);
    assert_eq!(
        (cut.outcome, cut.raw, cut.error),
        (UsageOutcome::Missing, None, None)
    );
}

#[test]
fn responses_error_event_is_recorded() {
    let body =
        b"event: error\ndata: {\"type\":\"error\",\"code\":\"x\",\"message\":\"overloaded\"}\n\n";
    let result = resp_usage(body);
    assert_eq!(result.outcome, UsageOutcome::Missing);
    assert_eq!(result.error.as_deref(), Some("overloaded"));
    let bare = resp_usage(b"data: {\"type\":\"error\"}\n\n");
    assert_eq!(bare.error.as_deref(), Some("{\"type\":\"error\"}"));
}

#[test]
fn responses_awkward_splits_give_identical_usage() {
    let expected = resp_usage(RESP_COMPLETED);
    assert_eq!(expected.outcome, UsageOutcome::Reported(resp_sse_usage()));
    for at in 0..=RESP_COMPLETED.len() {
        let mut decoder = SseDecoder::default();
        let mut events = decoder.push(&RESP_COMPLETED[..at]);
        events.extend(decoder.push(&RESP_COMPLETED[at..]));
        assert_eq!(
            OpenAi.usage(&ResponseView::Events(events)),
            expected,
            "split at {at}"
        );
    }
}

#[test]
fn responses_segments_follow_order_with_labels() {
    let segments = OpenAi.segments(&resp_request());
    let labels: Vec<&str> = segments.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "tools[0] get_weather",
            "tools[1] web_search",
            "instructions",
            "input[0] developer/text",
            "input[1].0 user/input_text",
            "input[1].1 user/input_text",
            "input[2] reasoning",
            "input[3] function_call",
            "input[4] function_call_output",
            "input[5] user/text",
        ]
    );
    let sources: Vec<SegmentSource> = segments.iter().map(|s| s.source).collect();
    assert!(sources.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(sources[3], SegmentSource::System);
    assert_eq!(sources[4], SegmentSource::Messages);
}

#[test]
fn responses_breakpoint_is_marked_and_stripped() {
    let segments = OpenAi.segments(&resp_request());
    assert_eq!(segments[4].breakpoint, Some(TWELVE_HOURS));
    assert!(segments[4].text.contains("prompt_cache_breakpoint"));
    assert!(!segments[4].content.contains("prompt_cache_breakpoint"));
    assert!(segments
        .iter()
        .enumerate()
        .all(|(i, s)| (i == 4) == s.breakpoint.is_some()));
    assert!(segments[7].content.contains("get_weather"));
}

#[test]
fn string_input_is_one_user_segment() {
    let segments = OpenAi.segments(&json!({"input": "hi", "instructions": "be brief"}));
    let labels: Vec<&str> = segments.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, ["instructions", "input user/text"]);
    assert_eq!(segments[1].source, SegmentSource::Messages);
}

#[test]
fn breakpoint_ttl_12h_is_its_own_tier_and_unknown_stays_unspecified() {
    let part = |ttl: &str| json!({"prompt_cache_breakpoint": {"mode": "explicit", "ttl": ttl}});
    assert_eq!(
        breakpoint(&part("12h")),
        Some(CacheTier::Ttl { seconds: 43_200 })
    );
    assert_eq!(breakpoint(&part("1d")), Some(CacheTier::Unspecified));
    assert_eq!(breakpoint(&part("30m")), Some(THIRTY_MIN));
}

#[test]
fn chat_tool_message_content_carries_the_tool_call_id() {
    let body = |id: &str| {
        json!({"messages": [
            {"role": "user", "content": "go"},
            {"role": "tool", "tool_call_id": id, "content": "same"},
            {"role": "tool", "tool_call_id": id, "content": [{"type": "text", "text": "same"}]},
        ]})
    };
    let a = OpenAi.segments(&body("call_1"));
    let b = OpenAi.segments(&body("call_2"));
    assert_eq!(a[0].content, b[0].content);
    assert_ne!(a[1].content, b[1].content);
    assert_ne!(a[2].content, b[2].content);
    assert!(a[1].content.contains("call_1"));
}

#[test]
fn classify_accepts_the_responses_path() {
    assert_eq!(
        OpenAi.classify(&head("POST", "/v1/responses")),
        CallKind::Inference
    );
    assert_eq!(
        OpenAi.classify(&head("POST", "/v1/responses?x=1")),
        CallKind::Inference
    );
    assert_eq!(
        OpenAi.classify(&head("GET", "/v1/responses")),
        CallKind::Other
    );
    assert_eq!(
        OpenAi.classify(&head("POST", "/v1/responses/resp_1/cancel")),
        CallKind::Other
    );
}

#[test]
fn responses_facts_include_the_responses_params() {
    let facts = OpenAi.request_facts(&resp_request());
    let keys: Vec<&str> = facts.params.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "tool_choice",
            "prompt_cache_key",
            "reasoning",
            "text",
            "store",
            "max_output_tokens",
            "include"
        ]
    );
    let chained = json!({"previous_response_id": "resp_0", "truncation": "auto"});
    let facts = OpenAi.request_facts(&chained);
    assert!(facts.params.contains_key("previous_response_id"));
    assert!(facts.params.contains_key("truncation"));
    assert!(OpenAi
        .cache_relevant_params()
        .contains(&"previous_response_id"));
}
