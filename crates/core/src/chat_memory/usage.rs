use crate::chat_process::ResetReason;
use crate::stream::{IterationCosts, IterationMetadata, ModelTotals};
use crate::wire::usage::{FIVE_MIN, ONE_HOUR};
use crate::worker::ModelUsage;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    User,
    Worker,
    #[serde(rename = "send_it")]
    SendIt,
}

pub fn trigger_entries(trigger: Trigger, texts: &[String]) -> Vec<(super::Role, String)> {
    let role = match trigger {
        Trigger::User | Trigger::SendIt => super::Role::User,
        Trigger::Worker => super::Role::Worker,
    };
    texts.iter().map(|text| (role, text.clone())).collect()
}

pub fn reply_entries(reply: &str) -> Vec<(super::Role, String)> {
    vec![(super::Role::Assistant, reply.to_string())]
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CallSite {
    Turn {
        trigger: Trigger,
    },
    Nap,
    Tag,
    Worker {
        worker_id: String,
        turn_id: Option<u64>,
    },
    Ping,
    Merge,
    Ask,
    Subagent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum LedgerRole {
    Main,
    Nap,
    Tag,
    Merge,
    Worker,
    Ask,
    Subagent,
}

impl CallSite {
    fn role(&self) -> LedgerRole {
        match self {
            CallSite::Turn { .. } | CallSite::Ping => LedgerRole::Main,
            CallSite::Nap => LedgerRole::Nap,
            CallSite::Tag => LedgerRole::Tag,
            CallSite::Merge => LedgerRole::Merge,
            CallSite::Worker { .. } => LedgerRole::Worker,
            CallSite::Ask => LedgerRole::Ask,
            CallSite::Subagent => LedgerRole::Subagent,
        }
    }
}

pub const VIEW_HEAD_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ViewStats {
    pub view_bytes: usize,
    pub view_lines: usize,
    pub view_head_hash: String,
    pub view_hash: String,
}

fn fnv1a(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

pub fn view_stats(lines: &[String]) -> ViewStats {
    let text = lines.join("\n");
    let bytes = text.as_bytes();
    ViewStats {
        view_bytes: bytes.len(),
        view_lines: lines.len(),
        view_head_hash: fnv1a(&bytes[..bytes.len().min(VIEW_HEAD_BYTES)]),
        view_hash: fnv1a(bytes),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CostSource {
    Harness,
}

pub(crate) const MAIN_HARNESS: &str = "claude";

pub fn carry_metadata(
    previous: &IterationMetadata,
    launched: bool,
    seen: &IterationMetadata,
) -> IterationMetadata {
    let base = if launched {
        IterationMetadata::new()
    } else {
        previous.clone()
    };
    IterationMetadata {
        session_id: seen.session_id.clone().or(base.session_id),
        model: seen.model.clone().or(base.model),
    }
}

#[derive(Debug, Serialize)]
pub struct LedgerRow {
    at: String,
    #[serde(flatten)]
    kind: CallSite,
    role: LedgerRole,
    input_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    output_tokens: u64,
    cost_usd: Option<f64>,
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    process_turn: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset: Option<ResetReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ticket: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_source: Option<CostSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_write_5m: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_write_1h: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    long_prompt: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    per_thread_usage: Option<bool>,
    #[serde(flatten)]
    view: Option<ViewStats>,
}

impl LedgerRow {
    pub fn with_process_turn(self, process_turn: u64) -> LedgerRow {
        LedgerRow {
            process_turn: Some(process_turn),
            ..self
        }
    }

    pub fn with_request_at(self, request_at: Option<String>) -> LedgerRow {
        LedgerRow { request_at, ..self }
    }

    pub fn with_reset(self, reset: Option<ResetReason>) -> LedgerRow {
        LedgerRow { reset, ..self }
    }

    pub fn with_run(self, run: Option<String>) -> LedgerRow {
        LedgerRow { run, ..self }
    }

    pub fn with_store(self, store: Option<String>) -> LedgerRow {
        LedgerRow { store, ..self }
    }

    pub fn with_ticket(self, ticket: Option<String>) -> LedgerRow {
        LedgerRow { ticket, ..self }
    }

    pub fn with_per_thread_usage(self) -> LedgerRow {
        LedgerRow {
            per_thread_usage: Some(true),
            ..self
        }
    }

    pub fn with_view(self, view: ViewStats) -> LedgerRow {
        LedgerRow {
            view: Some(view),
            ..self
        }
    }

    pub fn with_harness_cost(self, cost_usd: Option<f64>) -> LedgerRow {
        LedgerRow {
            cost_usd,
            cost_source: cost_usd.map(|_| CostSource::Harness),
            ..self
        }
    }
}

pub fn usage_record(
    kind: CallSite,
    at: String,
    costs: &IterationCosts,
    metadata: &IterationMetadata,
) -> Option<LedgerRow> {
    let usage = costs.usage.as_ref()?;
    Some(LedgerRow {
        at,
        role: kind.role(),
        kind,
        input_tokens: usage.input_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens.unwrap_or(0),
        cache_read_input_tokens: usage.cache_read_input_tokens.unwrap_or(0),
        output_tokens: usage.output_tokens,
        cost_usd: costs.cost_usd,
        duration_ms: costs.duration_ms,
        context_tokens: costs.context_tokens,
        process_turn: None,
        request_at: None,
        reset: None,
        run: None,
        store: None,
        ticket: None,
        harness: Some(MAIN_HARNESS.to_string()),
        model: metadata.model.clone(),
        session_id: metadata.session_id.clone(),
        cost_source: costs.cost_usd.map(|_| CostSource::Harness),
        reasoning_tokens: usage
            .output_tokens_details
            .as_ref()
            .and_then(|details| details.thinking_tokens),
        cache_write_5m: usage
            .cache_creation
            .as_ref()
            .map(|tiers| tiers.ephemeral_5m_input_tokens),
        cache_write_1h: usage
            .cache_creation
            .as_ref()
            .map(|tiers| tiers.ephemeral_1h_input_tokens),
        long_prompt: None,
        per_thread_usage: None,
        view: None,
    })
}

pub fn worker_usage_record(
    kind: CallSite,
    at: String,
    session_id: Option<String>,
    harness: &str,
    row: &ModelUsage,
) -> LedgerRow {
    let usage = &row.usage;
    let written = |tier| {
        usage
            .cache_writes
            .iter()
            .find(|write| write.tier == tier)
            .map(|write| write.tokens)
    };
    LedgerRow {
        at,
        role: kind.role(),
        kind,
        input_tokens: usage.input_uncached,
        cache_creation_input_tokens: usage.cache_writes.iter().map(|write| write.tokens).sum(),
        cache_read_input_tokens: usage.cache_read,
        output_tokens: usage.output,
        cost_usd: None,
        duration_ms: None,
        context_tokens: None,
        process_turn: None,
        request_at: None,
        reset: None,
        run: None,
        store: None,
        ticket: None,
        harness: Some(harness.to_string()),
        model: Some(row.model.clone()),
        session_id,
        cost_source: None,
        reasoning_tokens: usage.reasoning,
        cache_write_5m: written(FIVE_MIN),
        cache_write_1h: written(ONE_HOUR),
        long_prompt: Some(row.long_prompt),
        per_thread_usage: None,
        view: None,
    }
}

pub fn model_usage_since(
    previous: &BTreeMap<String, ModelTotals>,
    current: &BTreeMap<String, ModelTotals>,
) -> BTreeMap<String, ModelTotals> {
    current
        .iter()
        .map(|(model, now)| {
            let before = previous.get(model).cloned().unwrap_or_default();
            (model.clone(), now.saturating_sub(&before))
        })
        .collect()
}

fn subagent_usage(totals: &ModelTotals) -> crate::wire::usage::Usage {
    use crate::wire::usage::{CacheTier, CacheWrite, Usage};
    let untiered = totals
        .cache_creation_input_tokens
        .saturating_sub(totals.cache_write_5m + totals.cache_write_1h);
    let writes = [
        (FIVE_MIN, totals.cache_write_5m),
        (ONE_HOUR, totals.cache_write_1h),
        (CacheTier::Unspecified, untiered),
    ];
    Usage {
        input_uncached: totals.input_tokens,
        cache_read: totals.cache_read_input_tokens,
        cache_writes: writes
            .into_iter()
            .filter(|(tier, tokens)| *tokens > 0 || *tier != CacheTier::Unspecified)
            .map(|(tier, tokens)| CacheWrite { tier, tokens })
            .collect(),
        output: totals.output_tokens,
        reasoning: Some(totals.thinking_tokens),
        other: Vec::new(),
    }
}

pub fn subagent_records(
    at: &str,
    costs: &IterationCosts,
    metadata: &IterationMetadata,
) -> Vec<LedgerRow> {
    let main_loop = costs
        .usage
        .as_ref()
        .map(ModelTotals::from)
        .unwrap_or_default();
    costs
        .model_usage
        .iter()
        .map(
            |(model, totals)| match metadata.model.as_deref() == Some(model) {
                true => (model, totals.saturating_sub(&main_loop)),
                false => (model, totals.clone()),
            },
        )
        .filter(|(_, left)| *left != ModelTotals::default())
        .map(|(model, left)| {
            let row = ModelUsage {
                model: model.clone(),
                usage: subagent_usage(&left),
                long_prompt: false,
            };
            LedgerRow {
                long_prompt: None,
                ..worker_usage_record(
                    CallSite::Subagent,
                    at.to_string(),
                    metadata.session_id.clone(),
                    MAIN_HARNESS,
                    &row,
                )
            }
        })
        .collect()
}

pub fn encode_usage(record: &LedgerRow) -> Result<String, String> {
    let mut line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
