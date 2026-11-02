use serde_json::Value;

use super::{
    CallKind, PrefixCache, PromptSegment, Provider, ProviderId, RequestFacts, RequestHead,
    ResponseUsage, ResponseView, SegmentSource,
};
use crate::wire::headers::StoredHeaders;
use crate::wire::price::{matches_prefix, normalize_model, PriceTable};
use crate::wire::usage::{BillingRates, CacheTier, CacheWrite, TierRate, Usage, THIRTY_MIN};

mod chat;
pub(crate) mod price;

#[cfg(test)]
mod tests;

const PARAM_KEYS: [&str; 9] = [
    "tool_choice",
    "reasoning_effort",
    "response_format",
    "prompt_cache_key",
    "prompt_cache_options",
    "prompt_cache_retention",
    "stream_options",
    "max_completion_tokens",
    "service_tier",
];

const DEFAULT_CACHE_READ: f64 = 0.1;
const DEFAULT_CACHE_WRITE: f64 = 1.25;
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
    written: Option<u64>,
    output: u64,
    reasoning: Option<u64>,
    other: Vec<(&'static str, Option<u64>)>,
}

fn build_usage(counts: Counts) -> Usage {
    let cached = counts.cached.unwrap_or(0);
    let written = counts.written.unwrap_or(0);
    Usage {
        input_uncached: counts
            .input_total
            .saturating_sub(cached)
            .saturating_sub(written),
        cache_read: cached,
        cache_writes: counts
            .written
            .map(|tokens| CacheWrite {
                tier: THIRTY_MIN,
                tokens,
            })
            .into_iter()
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

fn ratios(model: &str) -> BillingRates {
    let model = normalize_model(model);
    let entries = price::entries();
    let listed = entries
        .iter()
        .filter(|entry| matches_prefix(model, &entry.model))
        .max_by_key(|entry| entry.model.len());
    let Some(entry) = listed.filter(|entry| entry.price.input > 0.0) else {
        return BillingRates {
            cache_read: DEFAULT_CACHE_READ,
            cache_writes: vec![TierRate {
                tier: THIRTY_MIN,
                multiplier: DEFAULT_CACHE_WRITE,
            }],
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
        ]
    }

    fn classify(&self, call: &RequestHead) -> CallKind {
        let path = call.path.split('?').next().unwrap_or_default();
        if call.method.eq_ignore_ascii_case("POST") && path == "/v1/chat/completions" {
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
        chat::segments(body)
    }

    fn prefix_cache(&self, body: &Value) -> PrefixCache {
        match body
            .pointer("/prompt_cache_options/mode")
            .and_then(Value::as_str)
        {
            Some("explicit") => PrefixCache::Explicit,
            _ => PrefixCache::Implicit { tier: THIRTY_MIN },
        }
    }

    fn usage(&self, response: &ResponseView) -> ResponseUsage {
        chat::usage(response)
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

    fn price_factor(&self, _raw_usage: &Value, _model: &str) -> Option<f64> {
        Some(1.0)
    }
}
