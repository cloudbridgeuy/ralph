use serde_json::Value;

use super::{
    CallKind, PrefixCache, PromptSegment, Provider, ProviderId, RequestFacts, RequestHead,
    ResponseUsage, ResponseView, SegmentSource,
};
use crate::wire::headers::StoredHeaders;
use crate::wire::price::{matches_prefix, normalize_model, PriceEntry, PriceTable};
use crate::wire::usage::{
    BillingRates, CacheTier, CacheWrite, TierRate, Usage, UsageOutcome, THIRTY_MIN, TWELVE_HOURS,
};

mod chat;
pub(crate) mod price;
mod responses;

#[cfg(test)]
mod tests;

const PARAM_KEYS: [&str; 16] = [
    "tool_choice",
    "reasoning_effort",
    "response_format",
    "prompt_cache_key",
    "prompt_cache_options",
    "prompt_cache_retention",
    "stream_options",
    "max_completion_tokens",
    "service_tier",
    "reasoning",
    "text",
    "previous_response_id",
    "store",
    "truncation",
    "max_output_tokens",
    "include",
];

const EXPLICIT_LOOKBACK_BLOCKS: usize = 1;
const MIN_CACHEABLE_TOKENS: u64 = 1_024;

const DEFAULT_CACHE_READ: f64 = 0.1;
const DEFAULT_CACHE_WRITE: f64 = 1.25;
const DEFAULT_CACHE_WRITE_12H: f64 = 2.5;
const DEFAULT_OUTPUT: f64 = 5.0;

pub struct OpenAi;

fn count(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
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

fn breakpoint(block: &Value) -> Option<CacheTier> {
    match block.get("prompt_cache_breakpoint") {
        None | Some(Value::Null) => None,
        Some(marker) => Some(match marker.get("ttl") {
            None => THIRTY_MIN,
            Some(Value::String(ttl)) if ttl == "30m" => THIRTY_MIN,
            Some(Value::String(ttl)) if ttl == "12h" => TWELVE_HOURS,
            Some(_) => CacheTier::Unspecified,
        }),
    }
}

fn without_marker(block: &Value) -> String {
    let mut bare = block.clone();
    if let Some(object) = bare.as_object_mut() {
        object.remove("prompt_cache_breakpoint");
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

struct Counts {
    input_total: u64,
    cached: Option<u64>,
    writes: Option<Vec<(CacheTier, u64)>>,
    output: u64,
    reasoning: Option<u64>,
    other: Vec<(&'static str, Option<u64>)>,
}

fn build_usage(counts: Counts) -> Usage {
    let cached = counts.cached.unwrap_or(0);
    let written: u64 = counts
        .writes
        .iter()
        .flatten()
        .map(|(_, tokens)| tokens)
        .sum();
    Usage {
        input_uncached: counts
            .input_total
            .saturating_sub(cached)
            .saturating_sub(written),
        cache_read: cached,
        cache_writes: counts
            .writes
            .into_iter()
            .flatten()
            .map(|(tier, tokens)| CacheWrite { tier, tokens })
            .collect(),
        output: counts.output,
        reasoning: counts.reasoning,
        other: counts
            .other
            .into_iter()
            .filter_map(|(name, tokens)| Some((name.to_string(), tokens.filter(|n| *n > 0)?)))
            .collect(),
    }
}

fn usage_object(value: &Value) -> Option<Value> {
    value
        .get("usage")
        .filter(|usage| usage.is_object())
        .cloned()
}

fn reported(
    raw: Option<Value>,
    error: Option<String>,
    map: impl Fn(&Value) -> Usage,
) -> ResponseUsage {
    let outcome = raw.as_ref().map_or(UsageOutcome::Missing, |raw| {
        UsageOutcome::Reported(map(raw))
    });
    ResponseUsage {
        raw,
        outcome,
        error,
    }
}

fn error_message(value: &Value) -> Option<String> {
    let error = value.get("error").filter(|error| error.is_object())?;
    Some(
        error
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .map_or_else(|| error.to_string(), String::from),
    )
}

fn implicit_tier(body: &Value) -> CacheTier {
    match body
        .pointer("/prompt_cache_options/ttl")
        .filter(|ttl| !ttl.is_null())
    {
        Some(Value::String(ttl)) if ttl == "30m" => THIRTY_MIN,
        Some(Value::String(ttl)) if ttl == "12h" => TWELVE_HOURS,
        Some(_) => CacheTier::Unspecified,
        None if body.get("prompt_cache_retention").is_some() => CacheTier::Unspecified,
        None if body
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|model| listed(model).is_some()) =>
        {
            THIRTY_MIN
        }
        None => CacheTier::Unspecified,
    }
}

fn listed(model: &str) -> Option<PriceEntry> {
    let model = normalize_model(model);
    price::entries()
        .into_iter()
        .filter(|entry| matches_prefix(model, &entry.model))
        .max_by_key(|entry| entry.model.len())
}

fn ratios(model: &str) -> BillingRates {
    let Some(entry) = listed(model).filter(|entry| entry.price.input > 0.0) else {
        return BillingRates {
            cache_read: DEFAULT_CACHE_READ,
            cache_writes: vec![
                TierRate {
                    tier: THIRTY_MIN,
                    multiplier: DEFAULT_CACHE_WRITE,
                },
                TierRate {
                    tier: TWELVE_HOURS,
                    multiplier: DEFAULT_CACHE_WRITE_12H,
                },
            ],
            output: DEFAULT_OUTPUT,
        };
    };
    let input = entry.price.input;
    BillingRates {
        cache_read: entry.price.cache_read / input,
        cache_writes: entry
            .price
            .cache_writes
            .iter()
            .map(|write| TierRate {
                tier: write.tier,
                multiplier: write.usd_per_mtok / input,
            })
            .collect(),
        output: entry.price.output / input,
    }
}

impl Provider for OpenAi {
    fn id(&self) -> ProviderId {
        ProviderId::OpenAi
    }

    fn default_upstream(&self) -> &'static str {
        "https://api.openai.com"
    }

    fn base_url_hint(&self, listen: &str) -> String {
        format!("export OPENAI_BASE_URL=http://{listen}/v1")
    }

    fn stored_response_headers(&self) -> StoredHeaders {
        StoredHeaders {
            names: &["x-request-id", "openai-processing-ms", "openai-version"],
            prefixes: &["x-ratelimit-"],
        }
    }

    fn secret_headers(&self) -> &'static [&'static str] {
        &[]
    }

    fn cache_relevant_headers(&self) -> &'static [&'static str] {
        &["openai-organization", "openai-project"]
    }

    fn cache_relevant_params(&self) -> &'static [&'static str] {
        &[
            "prompt_cache_key",
            "prompt_cache_options",
            "prompt_cache_retention",
            "response_format",
            "reasoning_effort",
            "tool_choice",
            "text",
            "reasoning",
            "previous_response_id",
        ]
    }

    fn classify(&self, call: &RequestHead) -> CallKind {
        let path = call.path.split('?').next().unwrap_or_default();
        let inference = matches!(path, "/v1/chat/completions" | "/v1/responses");
        if call.method.eq_ignore_ascii_case("POST") && inference {
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
        if body.get("input").is_some() || body.get("instructions").is_some() {
            responses::segments(body)
        } else {
            chat::segments(body)
        }
    }

    fn prefix_cache(&self, body: &Value) -> PrefixCache {
        if body
            .pointer("/prompt_cache_options/mode")
            .and_then(Value::as_str)
            == Some("explicit")
        {
            return PrefixCache::Explicit {
                lookback: EXPLICIT_LOOKBACK_BLOCKS,
            };
        }
        PrefixCache::Implicit {
            tier: implicit_tier(body),
            min_tokens: body
                .get("model")
                .and_then(Value::as_str)
                .and_then(listed)
                .map(|_| MIN_CACHEABLE_TOKENS),
        }
    }

    fn usage(&self, response: &ResponseView) -> ResponseUsage {
        match response {
            ResponseView::Json(value) if responses::is_response(value) => {
                responses::json_usage(value)
            }
            ResponseView::Json(value) => chat::json_usage(value),
            ResponseView::Events(events) if responses::is_stream(events) => {
                responses::event_usage(events)
            }
            ResponseView::Events(events) => chat::event_usage(events),
        }
    }

    fn rates(&self, model: &str) -> BillingRates {
        ratios(model)
    }

    fn prices(&self) -> PriceTable {
        PriceTable {
            as_of: price::AS_OF,
            entries: price::entries(),
        }
    }

    fn price_factor(&self, _raw_usage: &Value, facts: &RequestFacts, _model: &str) -> Option<f64> {
        match facts.params.get("service_tier") {
            None | Some(Value::Null) => Some(1.0),
            Some(Value::String(tier)) if matches!(tier.as_str(), "default" | "auto") => Some(1.0),
            Some(_) => None,
        }
    }
}
