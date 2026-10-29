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

    fn since(self, earlier: Self) -> Self {
        Self {
            input_uncached: self.input_uncached.saturating_sub(earlier.input_uncached),
            cache_read: self.cache_read.saturating_sub(earlier.cache_read),
            cache_write: self.cache_write.saturating_sub(earlier.cache_write),
            output: self.output.saturating_sub(earlier.output),
            reasoning: self.reasoning.saturating_sub(earlier.reasoning),
        }
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
    Call { total: CallTokens, last: CallTokens },
    Rerouted(String),
}

fn count(breakdown: &Value, key: &str) -> u64 {
    breakdown[key].as_u64().unwrap_or(0)
}

fn decode_call(last: &Value) -> CallTokens {
    let cached = count(last, "cachedInputTokens");
    let written = count(last, "cacheWriteInputTokens");
    CallTokens {
        input_uncached: count(last, "inputTokens")
            .saturating_sub(cached)
            .saturating_sub(written),
        cache_read: cached,
        cache_write: written,
        output: count(last, "outputTokens"),
        reasoning: count(last, "reasoningOutputTokens"),
    }
}

pub fn decode_usage_event(method: &str, params: &Value) -> Option<UsageEvent> {
    match method {
        TOKEN_USAGE_UPDATED => {
            let usage = &params["tokenUsage"];
            (usage["total"].is_object() && usage["last"].is_object()).then(|| UsageEvent::Call {
                total: decode_call(&usage["total"]),
                last: decode_call(&usage["last"]),
            })
        }
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
    seen_total: Option<CallTokens>,
    by_model: BTreeMap<String, CallTokens>,
}

impl TurnUsage {
    pub fn new(model: Option<String>) -> Self {
        Self {
            base: model.clone(),
            current: model,
            seen_total: None,
            by_model: BTreeMap::new(),
        }
    }

    pub fn observe(&mut self, event: UsageEvent, in_turn: bool) {
        match event {
            UsageEvent::Rerouted(model) if in_turn => self.current = Some(model),
            UsageEvent::Rerouted(_) => {}
            UsageEvent::Call { total, last } => {
                let tokens = self.seen_total.map_or(last, |seen| total.since(seen));
                self.seen_total = Some(total);
                if !in_turn {
                    return;
                }
                let model = self.current.as_deref().unwrap_or(UNKNOWN_MODEL);
                self.by_model
                    .entry(model.to_string())
                    .or_default()
                    .add(tokens);
            }
        }
    }

    pub fn start_turn(&mut self) {
        self.current = self.base.clone();
    }

    pub fn take(&mut self) -> Vec<ModelUsage> {
        self.current = self.base.clone();
        std::mem::take(&mut self.by_model)
            .into_iter()
            .filter(|(_, tokens)| *tokens != CallTokens::default())
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

    fn updated(last: Value, total: Value) -> Value {
        json!({"threadId": "t", "turnId": "u", "tokenUsage": {"last": last, "total": total}})
    }

    fn breakdown(input: u64, output: u64) -> Value {
        json!({"inputTokens": input, "outputTokens": output})
    }

    fn call(input: u64, cached: u64, output: u64) -> UsageEvent {
        let last = json!({"inputTokens": input, "cachedInputTokens": cached, "outputTokens": output, "reasoningOutputTokens": 1});
        decode_usage_event(TOKEN_USAGE_UPDATED, &updated(last, json!({}))).unwrap()
    }

    fn totals(last: (u64, u64), total: (u64, u64)) -> UsageEvent {
        decode_usage_event(
            TOKEN_USAGE_UPDATED,
            &updated(breakdown(last.0, last.1), breakdown(total.0, total.1)),
        )
        .unwrap()
    }

    #[test]
    fn the_last_breakdown_maps_input_minus_cached_and_written() {
        let params = updated(
            json!({"inputTokens": 100, "cachedInputTokens": 30, "cacheWriteInputTokens": 5, "outputTokens": 7, "reasoningOutputTokens": 2, "totalTokens": 107}),
            json!({}),
        );
        let Some(UsageEvent::Call { last: tokens, .. }) =
            decode_usage_event(TOKEN_USAGE_UPDATED, &params)
        else {
            panic!("not a call");
        };
        let usage = tokens.usage();
        assert_eq!(usage.input_uncached, 65);
        assert_eq!(usage.cache_read, 30);
        assert_eq!(usage.cache_writes[0].tokens, 5);
        assert_eq!(usage.output, 7);
        assert_eq!(usage.reasoning, Some(2));
    }

    #[test]
    fn cache_writes_inside_input_are_not_counted_as_input() {
        let params = updated(
            json!({"inputTokens": 100, "cachedInputTokens": 40, "cacheWriteInputTokens": 60}),
            json!({}),
        );
        let Some(UsageEvent::Call { last: tokens, .. }) =
            decode_usage_event(TOKEN_USAGE_UPDATED, &params)
        else {
            panic!("not a call");
        };
        let usage = tokens.usage();
        assert_eq!(usage.input_uncached, 0);
        assert_eq!(usage.cache_read, 40);
        assert_eq!(usage.cache_writes[0].tokens, 60);
    }

    #[test]
    fn cached_above_input_gives_zero_input_and_a_missing_cache_write_gives_none() {
        let UsageEvent::Call { last: tokens, .. } = call(10, 40, 3) else {
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
        let no_total = json!({"tokenUsage": {"last": breakdown(1, 1)}});
        assert_eq!(decode_usage_event(TOKEN_USAGE_UPDATED, &no_total), None);
        assert_eq!(
            decode_usage_event(MODEL_REROUTED, &json!({"toModel": "b"})),
            Some(UsageEvent::Rerouted("b".into()))
        );
    }

    #[test]
    fn calls_sum_per_model_and_a_reroute_switches_the_model() {
        let mut turn = TurnUsage::new(Some("a".into()));
        turn.observe(totals((10, 1), (10, 1)), true);
        turn.observe(totals((5, 2), (15, 3)), true);
        turn.observe(UsageEvent::Rerouted("b".into()), true);
        turn.observe(totals((7, 3), (22, 6)), true);
        let rows = turn.take();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].model.as_str(), rows[0].usage.input_uncached),
            ("a", 15)
        );
        assert_eq!(rows[0].usage.output, 3);
        assert_eq!(
            (rows[1].model.as_str(), rows[1].usage.input_uncached),
            ("b", 7)
        );
    }

    #[test]
    fn take_empties_the_turn_and_restores_the_starting_model() {
        let mut turn = TurnUsage::new(Some("a".into()));
        turn.observe(UsageEvent::Rerouted("b".into()), true);
        turn.observe(totals((1, 1), (1, 1)), true);
        assert_eq!(turn.take().len(), 1);
        assert!(turn.take().is_empty());
        turn.observe(totals((1, 1), (2, 2)), true);
        assert_eq!(turn.take()[0].model, "a");
    }

    #[test]
    fn a_resent_update_counts_once_and_later_calls_count_the_total_delta() {
        let mut turn = TurnUsage::new(Some("a".into()));
        turn.observe(totals((10, 1), (50, 4)), true);
        turn.observe(totals((10, 1), (50, 4)), true);
        turn.observe(totals((5, 2), (55, 6)), true);
        let rows = turn.take();
        assert_eq!(
            (rows[0].usage.input_uncached, rows[0].usage.output),
            (15, 3)
        );
    }

    #[test]
    fn a_smaller_total_counts_zero() {
        let mut turn = TurnUsage::new(Some("a".into()));
        turn.observe(totals((10, 1), (10, 1)), true);
        turn.observe(totals((99, 9), (5, 0)), true);
        turn.observe(totals((99, 9), (7, 3)), true);
        let rows = turn.take();
        assert_eq!(
            (rows[0].usage.input_uncached, rows[0].usage.output),
            (12, 4)
        );
    }

    #[test]
    fn usage_between_turns_moves_the_baseline_but_is_not_counted() {
        let mut turn = TurnUsage::new(Some("a".into()));
        turn.observe(totals((10, 1), (10, 1)), false);
        turn.observe(UsageEvent::Rerouted("b".into()), false);
        assert!(turn.take().is_empty());
        turn.observe(totals((5, 1), (15, 2)), true);
        let rows = turn.take();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].model.as_str(), rows[0].usage.input_uncached),
            ("a", 5)
        );
    }

    #[test]
    fn a_turn_with_only_zero_usage_gives_no_row() {
        let mut turn = TurnUsage::new(Some("a".into()));
        turn.observe(totals((0, 0), (0, 0)), true);
        assert!(turn.take().is_empty());
        turn.observe(totals((4, 1), (4, 1)), true);
        assert_eq!(turn.take().len(), 1);
        turn.observe(totals((4, 1), (4, 1)), true);
        assert!(turn.take().is_empty());
    }

    #[test]
    fn a_new_turn_starts_on_the_base_model() {
        let mut turn = TurnUsage::new(Some("a".into()));
        turn.observe(UsageEvent::Rerouted("b".into()), true);
        turn.start_turn();
        turn.observe(totals((1, 1), (1, 1)), true);
        assert_eq!(turn.take()[0].model, "a");
    }
}
