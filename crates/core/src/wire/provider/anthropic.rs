use serde_json::{json, Map, Value};

use super::{
    CallKind, PrefixCache, PromptSegment, Provider, ProviderId, RequestFacts, RequestHead,
    ResponseUsage, ResponseView, SegmentSource,
};
use crate::wire::headers::StoredHeaders;
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

pub(crate) fn map_usage(raw: &Value) -> Usage {
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
        None | Some("global" | "not_available") => Some(1.0),
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

    fn base_url_hint(&self, listen: &str) -> String {
        format!("export ANTHROPIC_BASE_URL=http://{listen}")
    }

    fn stored_response_headers(&self) -> StoredHeaders {
        StoredHeaders {
            names: &["request-id"],
            prefixes: &["anthropic-ratelimit-"],
        }
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

    fn prefix_cache(&self, _body: &Value) -> PrefixCache {
        PrefixCache::Explicit
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
mod tests;
