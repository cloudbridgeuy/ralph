use crate::wire::usage::{CacheTier, CacheWrite, Usage};
use crate::worker::ModelUsage;
use serde_json::Value;
use std::collections::BTreeMap;

pub const TOKEN_USAGE_UPDATED: &str = "thread/tokenUsage/updated";
pub const MODEL_REROUTED: &str = "model/rerouted";
const UNKNOWN_MODEL: &str = "unknown";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallTokens {
    input_uncached: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
    reasoning: u64,
}

impl CallTokens {
    fn add(&mut self, other: Self) {
        self.input_uncached += other.input_uncached;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.output += other.output;
        self.reasoning += other.reasoning;
    }

    fn usage(self) -> Usage {
        Usage {
            input_uncached: self.input_uncached,
            cache_read: self.cache_read,
            cache_writes: Some(self.cache_write)
                .filter(|tokens| *tokens > 0)
                .map(|tokens| {
                    vec![CacheWrite {
                        tier: CacheTier::Unspecified,
                        tokens,
                    }]
                })
                .unwrap_or_default(),
            output: self.output,
            reasoning: Some(self.reasoning),
            other: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageEvent {
    Call(CallTokens),
    Rerouted(String),
}

fn count(breakdown: &Value, key: &str) -> u64 {
    breakdown[key].as_u64().unwrap_or(0)
}

fn decode_call(last: &Value) -> CallTokens {
    let cached = count(last, "cachedInputTokens");
    CallTokens {
        input_uncached: count(last, "inputTokens").saturating_sub(cached),
        cache_read: cached,
        cache_write: count(last, "cacheWriteInputTokens"),
        output: count(last, "outputTokens"),
        reasoning: count(last, "reasoningOutputTokens"),
    }
}

pub fn decode_usage_event(method: &str, params: &Value) -> Option<UsageEvent> {
    match method {
        TOKEN_USAGE_UPDATED => params["tokenUsage"]["last"]
            .is_object()
            .then(|| UsageEvent::Call(decode_call(&params["tokenUsage"]["last"]))),
        MODEL_REROUTED => params["toModel"]
            .as_str()
            .map(|model| UsageEvent::Rerouted(model.to_string())),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct TurnUsage {
    base: Option<String>,
    current: Option<String>,
    by_model: BTreeMap<String, CallTokens>,
}

impl TurnUsage {
    pub fn new(model: Option<String>) -> Self {
        Self {
            base: model.clone(),
            current: model,
            by_model: BTreeMap::new(),
        }
    }

    pub fn observe(&mut self, event: UsageEvent) {
        match event {
            UsageEvent::Rerouted(model) => self.current = Some(model),
            UsageEvent::Call(tokens) => {
                let model = self.current.as_deref().unwrap_or(UNKNOWN_MODEL);
                self.by_model
                    .entry(model.to_string())
                    .or_default()
                    .add(tokens);
            }
        }
    }

    pub fn take(&mut self) -> Vec<ModelUsage> {
        self.current = self.base.clone();
        std::mem::take(&mut self.by_model)
            .into_iter()
            .map(|(model, tokens)| ModelUsage {
                model,
                usage: tokens.usage(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn updated(last: Value) -> Value {
        json!({"threadId": "t", "turnId": "u", "tokenUsage": {"last": last, "total": {}}})
    }

    fn call(input: u64, cached: u64, output: u64) -> UsageEvent {
        decode_usage_event(
            TOKEN_USAGE_UPDATED,
            &updated(json!({"inputTokens": input, "cachedInputTokens": cached, "outputTokens": output, "reasoningOutputTokens": 1})),
        )
        .unwrap()
    }

    #[test]
    fn the_last_breakdown_maps_input_minus_cached() {
        let params = updated(
            json!({"inputTokens": 100, "cachedInputTokens": 30, "cacheWriteInputTokens": 5, "outputTokens": 7, "reasoningOutputTokens": 2, "totalTokens": 107}),
        );
        let Some(UsageEvent::Call(tokens)) = decode_usage_event(TOKEN_USAGE_UPDATED, &params)
        else {
            panic!("not a call");
        };
        let usage = tokens.usage();
        assert_eq!(usage.input_uncached, 70);
        assert_eq!(usage.cache_read, 30);
        assert_eq!(usage.cache_writes[0].tokens, 5);
        assert_eq!(usage.output, 7);
        assert_eq!(usage.reasoning, Some(2));
    }

    #[test]
    fn cached_above_input_gives_zero_input_and_a_missing_cache_write_gives_none() {
        let UsageEvent::Call(tokens) = call(10, 40, 3) else {
            panic!("not a call");
        };
        let usage = tokens.usage();
        assert_eq!(usage.input_uncached, 0);
        assert!(usage.cache_writes.is_empty());
    }

    #[test]
    fn other_methods_and_missing_breakdowns_are_not_usage() {
        assert_eq!(decode_usage_event("turn/started", &json!({})), None);
        assert_eq!(decode_usage_event(TOKEN_USAGE_UPDATED, &json!({})), None);
        assert_eq!(
            decode_usage_event(MODEL_REROUTED, &json!({"toModel": "b"})),
            Some(UsageEvent::Rerouted("b".into()))
        );
    }

    #[test]
    fn calls_sum_per_model_and_a_reroute_switches_the_model() {
        let mut turn = TurnUsage::new(Some("a".into()));
        turn.observe(call(10, 0, 1));
        turn.observe(call(5, 0, 2));
        turn.observe(UsageEvent::Rerouted("b".into()));
        turn.observe(call(7, 0, 3));
        let rows = turn.take();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].model.as_str(), rows[0].usage.input_uncached),
            ("a", 15)
        );
        assert_eq!(
            (rows[0].usage.output, rows[0].usage.reasoning),
            (3, Some(2))
        );
        assert_eq!(
            (rows[1].model.as_str(), rows[1].usage.input_uncached),
            ("b", 7)
        );
    }

    #[test]
    fn take_empties_the_turn_and_restores_the_starting_model() {
        let mut turn = TurnUsage::new(Some("a".into()));
        turn.observe(UsageEvent::Rerouted("b".into()));
        turn.observe(call(1, 0, 1));
        assert_eq!(turn.take().len(), 1);
        assert!(turn.take().is_empty());
        turn.observe(call(1, 0, 1));
        assert_eq!(turn.take()[0].model, "a");
    }
}
