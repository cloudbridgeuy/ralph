use super::ServerEvent;
use crate::chat_memory::{worker_usage_record, CallKind, UsageRecord};
use crate::wire::usage::{CacheTier, CacheWrite, Usage};
use crate::worker::{ModelUsage, OPENCODE_HARNESS};
use serde::Deserialize;
use std::collections::BTreeMap;

const STEP_STARTED: &str = "session.step.started";
const STEP_ENDED: &str = "session.step.ended";
const STEP_FAILED: &str = "session.step.failed";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
struct StepCache {
    read: u64,
    write: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct StepTokens {
    input: u64,
    output: u64,
    reasoning: u64,
    cache: StepCache,
}

impl StepTokens {
    fn add(&mut self, other: Self) {
        self.input += other.input;
        self.output += other.output;
        self.reasoning += other.reasoning;
        self.cache.read += other.cache.read;
        self.cache.write += other.cache.write;
    }

    fn usage(self) -> Usage {
        let cache_writes = if self.cache.write > 0 {
            vec![CacheWrite {
                tier: CacheTier::Unspecified,
                tokens: self.cache.write,
            }]
        } else {
            Vec::new()
        };
        Usage {
            input_uncached: self.input,
            cache_read: self.cache.read,
            cache_writes,
            output: self.output + self.reasoning,
            reasoning: Some(self.reasoning),
            other: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpencodeUsageEvent {
    Model(String),
    Step { tokens: StepTokens, cost: f64 },
}

pub fn decode_opencode_usage(event: &ServerEvent) -> Option<OpencodeUsageEvent> {
    match event.kind.as_str() {
        STEP_STARTED => {
            let model = &event.data["model"];
            let id = model["id"].as_str()?;
            let name = match model["providerID"].as_str() {
                Some(provider) => format!("{provider}/{id}"),
                None => id.to_string(),
            };
            Some(OpencodeUsageEvent::Model(name))
        }
        STEP_ENDED | STEP_FAILED => {
            let tokens = serde_json::from_value(event.data.get("tokens")?.clone()).ok()?;
            Some(OpencodeUsageEvent::Step {
                tokens,
                cost: event.data["cost"].as_f64().unwrap_or(0.0),
            })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpencodeModelUsage {
    pub row: ModelUsage,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Spend {
    tokens: StepTokens,
    cost: f64,
}

#[derive(Debug, Default)]
pub struct OpencodeTurnUsage {
    model: String,
    by_model: BTreeMap<String, Spend>,
}

impl OpencodeTurnUsage {
    pub fn new(configured: &str) -> Self {
        Self {
            model: configured.to_string(),
            by_model: BTreeMap::new(),
        }
    }

    pub fn observe(&mut self, event: OpencodeUsageEvent) {
        match event {
            OpencodeUsageEvent::Model(model) => self.model = model,
            OpencodeUsageEvent::Step { tokens, cost } => {
                let spend = self.by_model.entry(self.model.clone()).or_default();
                spend.tokens.add(tokens);
                spend.cost += cost;
            }
        }
    }

    pub fn take(&mut self) -> Vec<OpencodeModelUsage> {
        std::mem::take(&mut self.by_model)
            .into_iter()
            .filter(|(_, spend)| spend.tokens != StepTokens::default())
            .map(|(model, spend)| OpencodeModelUsage {
                row: ModelUsage {
                    model,
                    usage: spend.tokens.usage(),
                },
                cost_usd: (spend.cost > 0.0).then_some(spend.cost),
            })
            .collect()
    }
}

pub fn opencode_usage_record(
    kind: CallKind,
    at: String,
    session_id: &str,
    spend: &OpencodeModelUsage,
) -> UsageRecord {
    worker_usage_record(
        kind,
        at,
        Some(session_id.to_string()),
        OPENCODE_HARNESS,
        &spend.row,
    )
    .with_harness_cost(spend.cost_usd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::parse_event;
    use serde_json::Value;

    const STARTED: &str = r#"data: {"id":"evt_1","type":"session.step.started","data":{"sessionID":"ses_a","assistantMessageID":"msg_1","model":{"id":"gpt-6.1-sol","providerID":"openai","variant":"default"}}}"#;

    const ENDED: &str = r#"data: {"id":"evt_2","type":"session.step.ended","data":{"sessionID":"ses_a","assistantMessageID":"msg_1","finish":"tool-calls","cost":0,"tokens":{"input":204,"output":23,"reasoning":40,"cache":{"read":12928,"write":0}}}}"#;

    const FAILED_WITH_TOKENS: &str = r#"data: {"id":"evt_3","type":"session.step.failed","data":{"sessionID":"ses_a","assistantMessageID":"msg_2","error":{"type":"aborted","message":"Step interrupted"},"cost":0.25,"tokens":{"input":10,"output":1,"reasoning":0,"cache":{"read":0,"write":7}}}}"#;

    const FAILED_BARE: &str = r#"data: {"id":"evt_4","type":"session.step.failed","data":{"sessionID":"ses_a","assistantMessageID":"msg_3","error":{"type":"aborted","message":"Step interrupted"}}}"#;

    fn decode(line: &str) -> Option<OpencodeUsageEvent> {
        decode_opencode_usage(&parse_event(line).unwrap())
    }

    fn observed(lines: &[&str]) -> OpencodeTurnUsage {
        let mut turn = OpencodeTurnUsage::new("openai/gpt-6.1-sol");
        for event in lines.iter().filter_map(|line| decode(line)) {
            turn.observe(event);
        }
        turn
    }

    #[test]
    fn decode_reads_the_step_model_and_tokens() {
        assert_eq!(
            decode(STARTED),
            Some(OpencodeUsageEvent::Model("openai/gpt-6.1-sol".into()))
        );
        let Some(OpencodeUsageEvent::Step { tokens, cost }) = decode(ENDED) else {
            panic!("not a step");
        };
        assert_eq!(tokens.input, 204);
        assert_eq!(tokens.reasoning, 40);
        assert_eq!(tokens.cache.read, 12928);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn decode_prefixes_the_step_model_with_its_provider() {
        assert_eq!(
            decode(STARTED),
            Some(OpencodeUsageEvent::Model("openai/gpt-6.1-sol".into()))
        );
    }

    #[test]
    fn decode_keeps_the_bare_id_without_a_provider() {
        let line = r#"data: {"id":"evt_1","type":"session.step.started","data":{"sessionID":"ses_a","model":{"id":"gpt-6.1-sol"}}}"#;
        assert_eq!(
            decode(line),
            Some(OpencodeUsageEvent::Model("gpt-6.1-sol".into()))
        );
    }

    #[test]
    fn a_failed_step_counts_only_with_tokens() {
        assert!(matches!(
            decode(FAILED_WITH_TOKENS),
            Some(OpencodeUsageEvent::Step { .. })
        ));
        assert_eq!(decode(FAILED_BARE), None);
    }

    #[test]
    fn other_events_are_not_usage() {
        let line =
            r#"data: {"id":"e","type":"session.execution.succeeded","data":{"sessionID":"ses_a"}}"#;
        assert_eq!(decode(line), None);
    }

    #[test]
    fn steps_accumulate_per_model_and_take_once() {
        let mut turn = observed(&[STARTED, ENDED, ENDED]);
        turn.observe(OpencodeUsageEvent::Model("gpt-6-luna".into()));
        turn.observe(decode(FAILED_WITH_TOKENS).unwrap());
        let rows = turn.take();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].row.model, "gpt-6-luna");
        assert_eq!(rows[0].row.usage.input_uncached, 10);
        assert_eq!(rows[1].row.model, "openai/gpt-6.1-sol");
        assert_eq!(rows[1].row.usage.input_uncached, 408);
        assert_eq!(rows[1].row.usage.cache_read, 25856);
        assert!(turn.take().is_empty());
    }

    #[test]
    fn steps_before_a_start_use_the_configured_model() {
        let rows = observed(&[ENDED]).take();
        assert_eq!(rows[0].row.model, "openai/gpt-6.1-sol");
    }

    #[test]
    fn take_skips_all_zero_spends() {
        let zero = r#"data: {"id":"evt_9","type":"session.step.ended","data":{"sessionID":"ses_a","assistantMessageID":"msg_9","finish":"stop","cost":0,"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}}}}"#;
        let mut turn = observed(&[STARTED, zero]);
        assert!(turn.take().is_empty());
    }

    #[test]
    fn output_includes_reasoning() {
        let rows = observed(&[STARTED, ENDED]).take();
        assert_eq!(rows[0].row.usage.output, 63);
        assert_eq!(rows[0].row.usage.reasoning, Some(40));
    }

    #[test]
    fn cost_is_harness_only_when_the_sum_is_positive() {
        let free = observed(&[STARTED, ENDED]).take();
        assert_eq!(free[0].cost_usd, None);
        let paid = observed(&[STARTED, FAILED_WITH_TOKENS, FAILED_WITH_TOKENS]).take();
        assert_eq!(paid[0].cost_usd, Some(0.5));
    }

    #[test]
    fn record_maps_every_field() {
        let rows = observed(&[STARTED, ENDED, FAILED_WITH_TOKENS]).take();
        let kind = CallKind::Worker {
            worker_id: "w1".into(),
            turn_id: Some(2),
        };
        let record = opencode_usage_record(kind, "t".into(), "ses_a", &rows[0]);
        let value: Value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["kind"], "worker");
        assert_eq!(value["worker_id"], "w1");
        assert_eq!(value["turn_id"], 2);
        assert_eq!(value["harness"], "opencode");
        assert_eq!(value["model"], "openai/gpt-6.1-sol");
        assert_eq!(value["session_id"], "ses_a");
        assert_eq!(value["input_tokens"], 214);
        assert_eq!(value["cache_read_input_tokens"], 12928);
        assert_eq!(value["cache_creation_input_tokens"], 7);
        assert_eq!(value["output_tokens"], 64);
        assert_eq!(value["reasoning_tokens"], 40);
        assert_eq!(value["cost_usd"], 0.25);
        assert_eq!(value["cost_source"], "harness");
    }

    #[test]
    fn record_without_cost_leaves_it_empty() {
        let rows = observed(&[STARTED, ENDED]).take();
        let kind = CallKind::Worker {
            worker_id: "w1".into(),
            turn_id: None,
        };
        let value =
            serde_json::to_value(opencode_usage_record(kind, "t".into(), "s", &rows[0])).unwrap();
        assert!(value["cost_usd"].is_null());
        assert!(value.get("cost_source").is_none());
    }
}
