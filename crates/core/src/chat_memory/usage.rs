use crate::stream::IterationCosts;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    User,
    Worker,
}

pub fn trigger_entries(trigger: Trigger, texts: &[String]) -> Vec<(super::Role, String)> {
    let role = match trigger {
        Trigger::User => super::Role::User,
        Trigger::Worker => super::Role::Worker,
    };
    texts.iter().map(|text| (role, text.clone())).collect()
}

pub fn reply_entries(trigger: Trigger, reply: &str) -> Vec<(super::Role, String)> {
    match trigger {
        Trigger::User | Trigger::Worker => vec![(super::Role::Assistant, reply.to_string())],
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CallKind {
    Turn { trigger: Trigger },
    Nap,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PromptBytes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frozen_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_bytes: Option<usize>,
    pub prompt_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_bytes: Option<usize>,
}

pub struct TurnParts<'a> {
    pub system: &'a str,
    pub frozen: &'a [String],
    pub tail: &'a [String],
    pub message: &'a str,
    pub prompt: &'a str,
    pub args: &'a [String],
}

pub fn turn_prompt_bytes(parts: &TurnParts) -> PromptBytes {
    PromptBytes {
        system_bytes: Some(parts.system.len()),
        frozen_bytes: Some(parts.frozen.join("\n").len()),
        tail_bytes: Some(parts.tail.join("\n").len()),
        message_bytes: Some(parts.message.len()),
        prompt_bytes: parts.prompt.len(),
        agents_bytes: parts
            .args
            .windows(2)
            .find(|pair| pair[0] == "--agents")
            .map(|pair| pair[1].len()),
        input_bytes: None,
    }
}

pub fn nap_prompt_bytes(prompt: &str) -> PromptBytes {
    PromptBytes {
        prompt_bytes: prompt.len(),
        input_bytes: prompt
            .find(super::nap::NAP_INPUT_MARKER)
            .map(|start| prompt.len() - start),
        ..PromptBytes::default()
    }
}

#[derive(Debug, Serialize)]
pub struct UsageRecord {
    pub at: String,
    #[serde(flatten)]
    pub kind: CallKind,
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    #[serde(flatten)]
    pub view: Option<ViewStats>,
    #[serde(flatten)]
    pub prompt: Option<PromptBytes>,
}

impl UsageRecord {
    pub fn with_view(self, view: ViewStats) -> UsageRecord {
        UsageRecord {
            view: Some(view),
            ..self
        }
    }

    pub fn with_prompt(self, prompt: PromptBytes) -> UsageRecord {
        UsageRecord {
            prompt: Some(prompt),
            ..self
        }
    }
}

pub fn usage_record(kind: CallKind, at: String, costs: &IterationCosts) -> Option<UsageRecord> {
    let usage = costs.usage.as_ref()?;
    Some(UsageRecord {
        at,
        kind,
        input_tokens: usage.input_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens.unwrap_or(0),
        cache_read_input_tokens: usage.cache_read_input_tokens.unwrap_or(0),
        output_tokens: usage.output_tokens,
        cost_usd: costs.cost_usd,
        duration_ms: costs.duration_ms,
        context_tokens: costs.context_tokens,
        view: None,
        prompt: None,
    })
}

pub fn encode_usage(record: &UsageRecord) -> Result<String, String> {
    let mut line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    line.push('\n');
    Ok(line)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn costs(cache: Option<u64>) -> IterationCosts {
        IterationCosts {
            cost_usd: Some(0.5),
            duration_ms: Some(40),
            usage: Some(crate::stream::Usage {
                input_tokens: 1,
                output_tokens: 2,
                cache_read_input_tokens: cache,
                cache_creation_input_tokens: cache.map(|c| c + 1),
            }),
            context_tokens: None,
        }
    }

    #[test]
    fn user_and_worker_turns_yield_log_entries() {
        let texts = ["a".to_string(), "b".to_string()];
        assert_eq!(
            trigger_entries(Trigger::User, &texts),
            vec![
                (crate::chat_memory::Role::User, "a".into()),
                (crate::chat_memory::Role::User, "b".into())
            ]
        );
        assert_eq!(
            trigger_entries(Trigger::Worker, &texts[..1]),
            vec![(crate::chat_memory::Role::Worker, "a".into())]
        );
        assert_eq!(
            reply_entries(Trigger::User, "r"),
            vec![(crate::chat_memory::Role::Assistant, "r".into())]
        );
    }

    #[test]
    fn usage_record_maps_all_fields() {
        let record = usage_record(CallKind::Nap, "t".into(), &costs(Some(7))).unwrap();
        assert_eq!(
            (
                record.input_tokens,
                record.output_tokens,
                record.cache_read_input_tokens,
                record.cache_creation_input_tokens
            ),
            (1, 2, 7, 8)
        );
        assert_eq!((record.cost_usd, record.duration_ms), (Some(0.5), Some(40)));
    }

    #[test]
    fn usage_record_zero_fills_missing_cache_fields() {
        assert!(usage_record(
            CallKind::Turn {
                trigger: Trigger::User
            },
            "t".into(),
            &IterationCosts::new()
        )
        .is_none());
        let record = usage_record(
            CallKind::Turn {
                trigger: Trigger::User,
            },
            "t".into(),
            &costs(None),
        )
        .unwrap();
        assert_eq!(
            (
                record.cache_read_input_tokens,
                record.cache_creation_input_tokens
            ),
            (0, 0)
        );
    }

    #[test]
    fn context_tokens_serialized_only_when_present() {
        let kind = CallKind::Nap;
        let mut with = costs(None);
        with.context_tokens = Some(42);
        let value = line(usage_record(kind, "t".into(), &with).unwrap());
        assert_eq!(value["context_tokens"], 42);
        let value = line(usage_record(kind, "t".into(), &costs(None)).unwrap());
        assert!(value.get("context_tokens").is_none());
    }

    #[test]
    fn encode_usage_is_one_json_line() {
        let record = usage_record(
            CallKind::Turn {
                trigger: Trigger::User,
            },
            "t".into(),
            &costs(None),
        )
        .unwrap();
        let line = encode_usage(&record).unwrap();
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["kind"], "turn");
        assert_eq!(value["trigger"], "user");
    }

    #[test]
    fn worker_turn_record_has_trigger() {
        let kind = CallKind::Turn {
            trigger: Trigger::Worker,
        };
        let record = usage_record(kind, "t".into(), &costs(None)).unwrap();
        let value: Value = serde_json::from_str(&encode_usage(&record).unwrap()).unwrap();
        assert_eq!(
            (&value["kind"], &value["trigger"]),
            (&"turn".into(), &"worker".into())
        );
    }

    #[test]
    fn nap_record_has_no_trigger() {
        let record = usage_record(CallKind::Nap, "t".into(), &costs(None)).unwrap();
        let value: Value = serde_json::from_str(&encode_usage(&record).unwrap()).unwrap();
        assert_eq!(value["kind"], "nap");
        assert!(value.get("trigger").is_none());
    }

    #[test]
    fn turn_record_carries_view_stats_and_nap_does_not() {
        let lines = vec!["#0-1 a".to_string(), "#2 b".to_string()];
        let stats = view_stats(&lines);
        assert_eq!((stats.view_bytes, stats.view_lines), (11, 2));
        let kind = CallKind::Turn {
            trigger: Trigger::User,
        };
        let record = usage_record(kind, "t".into(), &costs(None))
            .unwrap()
            .with_view(stats.clone());
        let value: Value = serde_json::from_str(&encode_usage(&record).unwrap()).unwrap();
        assert_eq!(value["view_bytes"], 11);
        assert_eq!(value["view_lines"], 2);
        assert_eq!(value["view_head_hash"], stats.view_head_hash.as_str());
        assert_eq!(value["view_hash"].as_str().map(str::len), Some(16));
        let nap = usage_record(CallKind::Nap, "t".into(), &costs(None)).unwrap();
        let value: Value = serde_json::from_str(&encode_usage(&nap).unwrap()).unwrap();
        assert!(value.get("view_bytes").is_none());
    }

    fn line(record: UsageRecord) -> Value {
        serde_json::from_str(&encode_usage(&record).unwrap()).unwrap()
    }

    #[test]
    fn turn_record_logs_prompt_parts_and_agents_only_when_present() {
        let frozen = vec!["ab".to_string(), "cd".to_string()];
        let tail = vec!["xyz".to_string()];
        let mut parts = TurnParts {
            system: "sys",
            frozen: &frozen,
            tail: &tail,
            message: "hello",
            prompt: "the whole prompt",
            args: &["--agents".to_string(), "{}".to_string()],
        };
        let kind = CallKind::Turn {
            trigger: Trigger::User,
        };
        let with = line(
            usage_record(kind, "t".into(), &costs(None))
                .unwrap()
                .with_prompt(turn_prompt_bytes(&parts)),
        );
        let got: Vec<_> = [
            "system_bytes",
            "frozen_bytes",
            "tail_bytes",
            "message_bytes",
            "prompt_bytes",
            "agents_bytes",
        ]
        .iter()
        .map(|key| with[key].as_u64())
        .collect();
        assert_eq!(got, [Some(3), Some(5), Some(3), Some(5), Some(16), Some(2)]);
        assert!(with.get("input_bytes").is_none());
        parts.args = &[];
        let without = line(
            usage_record(kind, "t".into(), &costs(None))
                .unwrap()
                .with_prompt(turn_prompt_bytes(&parts)),
        );
        assert!(without.get("agents_bytes").is_none());
    }

    #[test]
    fn nap_record_logs_input_bytes_from_the_marker_and_no_turn_keys() {
        let prompt = format!(
            "intro\n\n{}\nbody\n</input>",
            super::super::nap::NAP_INPUT_MARKER
        );
        let value = line(
            usage_record(CallKind::Nap, "t".into(), &costs(None))
                .unwrap()
                .with_prompt(nap_prompt_bytes(&prompt)),
        );
        assert_eq!(value["prompt_bytes"], prompt.len());
        assert_eq!(value["input_bytes"], prompt.len() - "intro\n\n".len());
        assert!(value.get("system_bytes").is_none() && value.get("agents_bytes").is_none());
        assert!(nap_prompt_bytes("no marker").input_bytes.is_none());
    }

    #[test]
    fn view_hash_is_stable_and_head_ignores_the_tail() {
        let short = vec!["a".repeat(VIEW_HEAD_BYTES)];
        let long = vec!["a".repeat(VIEW_HEAD_BYTES), "tail".to_string()];
        let (s, l) = (view_stats(&short), view_stats(&long));
        assert_eq!(s.view_head_hash, l.view_head_hash);
        assert_ne!(s.view_hash, l.view_hash);
        assert_eq!(view_stats(&[]).view_hash, "cbf29ce484222325");
    }
}
