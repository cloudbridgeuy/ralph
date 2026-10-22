use serde_json::{json, Map, Value};

use super::{
    CallKind, PromptSegment, Provider, ProviderId, RequestFacts, RequestHead, ResponseUsage,
    ResponseView, SegmentSource,
};
use crate::wire::price::{
    matches_prefix, normalize_model, ModelPrice, PriceEntry, PriceTable, TierPrice,
};
use crate::wire::sse::SseEvent;
use crate::wire::usage::{
    BillingRates, CacheTier, CacheWrite, TierRate, Usage, UsageOutcome, FIVE_MIN, ONE_HOUR,
};

const PARAM_KEYS: [&str; 6] = [
    "thinking",
    "tool_choice",
    "max_tokens",
    "output_config",
    "context_management",
    "metadata",
];

pub struct Anthropic;

fn count(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn cache_writes(raw: &Value) -> Vec<CacheWrite> {
    if let Some(breakdown) = raw.get("cache_creation").filter(|v| v.is_object()) {
        return [
            (FIVE_MIN, "ephemeral_5m_input_tokens"),
            (ONE_HOUR, "ephemeral_1h_input_tokens"),
        ]
        .into_iter()
        .filter_map(|(tier, key)| count(breakdown, key).map(|tokens| CacheWrite { tier, tokens }))
        .filter(|write| write.tokens > 0)
        .collect();
    }
    match count(raw, "cache_creation_input_tokens") {
        Some(tokens) if tokens > 0 => vec![CacheWrite {
            tier: CacheTier::Unspecified,
            tokens,
        }],
        _ => Vec::new(),
    }
}

fn server_tool_use(raw: &Value) -> Vec<(String, u64)> {
    let mut other: Vec<(String, u64)> = raw
        .get("server_tool_use")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(name, n)| n.as_u64().map(|n| (format!("server_tool_use.{name}"), n)))
        .collect();
    other.sort();
    other
}

fn map_usage(raw: &Value) -> Usage {
    Usage {
        input_uncached: count(raw, "input_tokens").unwrap_or(0),
        cache_read: count(raw, "cache_read_input_tokens").unwrap_or(0),
        cache_writes: cache_writes(raw),
        output: count(raw, "output_tokens").unwrap_or(0),
        reasoning: raw
            .get("output_tokens_details")
            .and_then(|details| count(details, "thinking_tokens")),
        other: server_tool_use(raw),
    }
}

fn error_message(value: &Value) -> Option<String> {
    if value.get("type").and_then(Value::as_str) != Some("error") {
        return None;
    }
    let message = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty());
    Some(match (message, value.get("error")) {
        (Some(message), _) => message.to_string(),
        (None, Some(error)) => error.to_string(),
        (None, None) => value.to_string(),
    })
}

fn json_usage(value: &Value) -> ResponseUsage {
    let raw = value.get("usage").filter(|v| v.is_object()).cloned();
    let outcome = raw.as_ref().map_or(UsageOutcome::Missing, |raw| {
        UsageOutcome::Reported(map_usage(raw))
    });
    ResponseUsage {
        raw,
        outcome,
        error: error_message(value),
    }
}

fn event_usage(events: &[SseEvent]) -> ResponseUsage {
    let mut start: Option<Map<String, Value>> = None;
    let mut deltas: Vec<Map<String, Value>> = Vec::new();
    let mut error = None;
    for event in events {
        let parsed = serde_json::from_str::<Value>(&event.data).ok();
        if error.is_none() {
            error = parsed
                .as_ref()
                .and_then(error_message)
                .or_else(|| (event.event.as_deref() == Some("error")).then(|| event.data.clone()));
        }
        let Some(data) = parsed else {
            continue;
        };
        match data.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(usage) = data.pointer("/message/usage").and_then(Value::as_object) {
                    start = Some(usage.clone());
                }
            }
            Some("message_delta") => {
                if let Some(usage) = data.get("usage").and_then(Value::as_object) {
                    deltas.push(usage.clone());
                }
            }
            _ => {}
        }
    }
    let has_deltas = !deltas.is_empty();
    let mut merged = start.unwrap_or_default();
    let has_start = !merged.is_empty();
    for delta in deltas {
        merged.extend(delta);
    }
    if !has_start && !has_deltas {
        return ResponseUsage {
            raw: None,
            outcome: UsageOutcome::Missing,
            error,
        };
    }
    let raw = Value::Object(merged);
    let usage = map_usage(&raw);
    let outcome = if has_deltas {
        UsageOutcome::Reported(usage)
    } else {
        UsageOutcome::Partial(usage)
    };
    ResponseUsage {
        raw: Some(raw),
        outcome,
        error,
    }
}

fn cache_read_multiplier(model: &str) -> f64 {
    let model = normalize_model(model);
    let is = |name: &str| matches_prefix(model, name);
    if is("claude-fable-5-1") || is("claude-mythos-5-1") {
        0.025
    } else if is("claude-opus-5-5") || is("claude-sonnet-5-5") {
        0.05
    } else {
        0.1
    }
}

const PRICE_AS_OF: &str = "2026-10-21";
const FAST_MODELS: [&str; 3] = ["claude-opus-5-5", "claude-opus-5", "claude-opus-4-8"];
const NO_GEO_MODELS: [&str; 2] = ["claude-opus-4-5", "claude-haiku-4-5"];

struct Row {
    model: &'static str,
    above_prompt_tokens: Option<u64>,
    input: f64,
    write_5m: f64,
    write_1h: f64,
    cache_read: f64,
    output: f64,
}

const fn row(
    model: &'static str,
    [input, write_5m, write_1h, cache_read, output]: [f64; 5],
) -> Row {
    Row {
        model,
        above_prompt_tokens: None,
        input,
        write_5m,
        write_1h,
        cache_read,
        output,
    }
}

const HAIKU_LONG_PROMPT: u64 = 100_000;

fn price_rows() -> Vec<Row> {
    vec![
        row("claude-fable-5-1", [10.0, 12.5, 20.0, 0.25, 50.0]),
        row("claude-fable-5", [10.0, 12.5, 20.0, 1.0, 50.0]),
        row("claude-opus-5-5", [4.0, 5.0, 8.0, 0.2, 20.0]),
        row("claude-opus-5", [5.0, 6.25, 10.0, 0.5, 25.0]),
        row("claude-opus-4-8", [5.0, 6.25, 10.0, 0.5, 25.0]),
        row("claude-opus-4-7", [5.0, 6.25, 10.0, 0.5, 25.0]),
        row("claude-opus-4-6", [5.0, 6.25, 10.0, 0.5, 25.0]),
        row("claude-opus-4-5", [5.0, 6.25, 10.0, 0.5, 25.0]),
        row("claude-sonnet-5-5", [2.0, 2.5, 4.0, 0.1, 10.0]),
        row("claude-sonnet-5", [2.0, 2.5, 4.0, 0.2, 10.0]),
        row("claude-sonnet-4-6", [3.0, 3.75, 6.0, 0.3, 15.0]),
        row("claude-haiku-5-5", [0.1, 0.125, 0.2, 0.01, 0.5]),
        Row {
            above_prompt_tokens: Some(HAIKU_LONG_PROMPT),
            ..row("claude-haiku-5-5", [0.5, 0.625, 1.0, 0.05, 2.5])
        },
        row("claude-haiku-4-5", [1.0, 1.25, 2.0, 0.1, 5.0]),
    ]
}

impl From<Row> for PriceEntry {
    fn from(row: Row) -> Self {
        Self {
            model: row.model.to_string(),
            above_prompt_tokens: row.above_prompt_tokens,
            price: ModelPrice {
                input: row.input,
                cache_read: row.cache_read,
                cache_writes: vec![
                    TierPrice {
                        tier: FIVE_MIN,
                        usd_per_mtok: row.write_5m,
                    },
                    TierPrice {
                        tier: ONE_HOUR,
                        usd_per_mtok: row.write_1h,
                    },
                ],
                output: row.output,
            },
        }
    }
}

fn speed_factor(raw: &Value, model: &str) -> Option<f64> {
    match raw.get("speed").and_then(Value::as_str) {
        None | Some("standard") => Some(1.0),
        Some("fast") if FAST_MODELS.iter().any(|m| matches_prefix(model, m)) => Some(2.0),
        Some(_) => None,
    }
}

fn geo_factor(raw: &Value, model: &str) -> Option<f64> {
    match raw.get("inference_geo").and_then(Value::as_str) {
        None | Some("global") => Some(1.0),
        Some("us") if !NO_GEO_MODELS.iter().any(|m| matches_prefix(model, m)) => Some(1.1),
        Some(_) => None,
    }
}

fn breakpoint(block: &Value) -> Option<CacheTier> {
    match block.get("cache_control") {
        None | Some(Value::Null) => None,
        Some(control) => Some(match control.get("ttl") {
            None => FIVE_MIN,
            Some(Value::String(ttl)) if ttl == "5m" => FIVE_MIN,
            Some(Value::String(ttl)) if ttl == "1h" => ONE_HOUR,
            Some(_) => CacheTier::Unspecified,
        }),
    }
}

fn without_marker(block: &Value) -> String {
    let mut bare = block.clone();
    if let Some(object) = bare.as_object_mut() {
        object.remove("cache_control");
    }
    bare.to_string()
}

fn segment(source: SegmentSource, label: String, block: &Value) -> PromptSegment {
    PromptSegment {
        label,
        source,
        text: block.to_string(),
        content: without_marker(block),
        breakpoint: breakpoint(block),
    }
}

fn items<'a>(body: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    body.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn text<'a>(value: &'a Value, key: &str, default: &'a str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or(default)
}

fn text_block(content: &Value) -> Value {
    json!({"type": "text", "text": content})
}

fn system_segments(body: &Value) -> Vec<PromptSegment> {
    match body.get("system") {
        Some(system @ Value::String(_)) => {
            vec![segment(
                SegmentSource::System,
                "system[0]".into(),
                &text_block(system),
            )]
        }
        _ => items(body, "system")
            .enumerate()
            .map(|(i, block)| segment(SegmentSource::System, format!("system[{i}]"), block))
            .collect(),
    }
}

fn message_segments(index: usize, message: &Value) -> Vec<PromptSegment> {
    let role = text(message, "role", "?");
    match message.get("content") {
        Some(content @ Value::String(_)) => {
            vec![segment(
                SegmentSource::Messages,
                format!("msg[{index}].0 {role}/text"),
                &text_block(content),
            )]
        }
        Some(Value::Array(blocks)) => blocks
            .iter()
            .enumerate()
            .map(|(j, block)| {
                let kind = text(block, "type", "?");
                segment(
                    SegmentSource::Messages,
                    format!("msg[{index}].{j} {role}/{kind}"),
                    block,
                )
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl Provider for Anthropic {
    fn id(&self) -> ProviderId {
        ProviderId::Anthropic
    }

    fn default_upstream(&self) -> &'static str {
        "https://api.anthropic.com"
    }

    fn base_url_env(&self) -> &'static str {
        "ANTHROPIC_BASE_URL"
    }

    fn secret_headers(&self) -> &'static [&'static str] {
        &[]
    }

    fn cache_relevant_headers(&self) -> &'static [&'static str] {
        &["anthropic-beta", "anthropic-version"]
    }

    fn cache_relevant_params(&self) -> &'static [&'static str] {
        &["thinking", "tool_choice"]
    }

    fn classify(&self, call: &RequestHead) -> CallKind {
        let path = call.path.split('?').next().unwrap_or_default();
        if call.method.eq_ignore_ascii_case("POST") && path == "/v1/messages" {
            CallKind::Inference
        } else {
            CallKind::Other
        }
    }

    fn request_facts(&self, body: &Value) -> RequestFacts {
        RequestFacts {
            model: body.get("model").and_then(Value::as_str).map(String::from),
            params: PARAM_KEYS
                .iter()
                .filter_map(|key| body.get(key).map(|v| (key.to_string(), v.clone())))
                .collect(),
        }
    }

    fn segments(&self, body: &Value) -> Vec<PromptSegment> {
        let tools = items(body, "tools").enumerate().map(|(i, tool)| {
            segment(
                SegmentSource::Tools,
                format!("tools[{i}] {}", text(tool, "name", "?")),
                tool,
            )
        });
        let messages = items(body, "messages")
            .enumerate()
            .flat_map(|(i, message)| message_segments(i, message));
        tools.chain(system_segments(body)).chain(messages).collect()
    }

    fn usage(&self, response: &ResponseView) -> ResponseUsage {
        match response {
            ResponseView::Json(value) => json_usage(value),
            ResponseView::Events(events) => event_usage(events),
        }
    }

    fn rates(&self, model: &str) -> BillingRates {
        BillingRates {
            cache_read: cache_read_multiplier(model),
            cache_writes: vec![
                TierRate {
                    tier: FIVE_MIN,
                    multiplier: 1.25,
                },
                TierRate {
                    tier: ONE_HOUR,
                    multiplier: 2.0,
                },
            ],
            output: 5.0,
        }
    }

    fn prices(&self) -> PriceTable {
        PriceTable {
            as_of: PRICE_AS_OF,
            entries: price_rows().into_iter().map(PriceEntry::from).collect(),
        }
    }

    fn price_factor(&self, raw_usage: &Value, model: &str) -> Option<f64> {
        let model = normalize_model(model);
        Some(speed_factor(raw_usage, model)? * geo_factor(raw_usage, model)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::provider::{assert_provider_contract, provider, ContractCase};
    use crate::wire::record::{CallRecord, RequestBody};
    use crate::wire::redact::redact_headers;
    use crate::wire::response::view;
    use crate::wire::sse::SseDecoder;
    use crate::wire::usage::input_equivalent;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    const GZIP: &[u8] = include_bytes!("fixtures/anthropic/haiku-ok-sse.gz");
    const JSON_USAGE: &[u8] = include_bytes!("fixtures/anthropic/json-usage.json");
    const DISAGREE: &[u8] = include_bytes!("fixtures/anthropic/iterations-disagree.sse");
    const PARTIAL: &[u8] = include_bytes!("fixtures/anthropic/partial.sse");
    const ERROR: &[u8] = include_bytes!("fixtures/anthropic/error.sse");
    const REQUEST: &str = include_str!("fixtures/anthropic/request.json");
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
        let sources: Vec<SegmentSource> =
            Anthropic.segments(&body).iter().map(|s| s.source).collect();
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
        let factor = |raw: Value, model: &str| Anthropic.price_factor(&raw, model);
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
        assert_eq!(Anthropic.base_url_env(), "ANTHROPIC_BASE_URL");
        assert!(Anthropic.secret_headers().is_empty());
    }
}
