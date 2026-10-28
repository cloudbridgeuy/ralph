use crate::chat_thread::ResetReason;
use crate::stream::{IterationCosts, IterationMetadata};
use crate::wire::usage::{FIVE_MIN, ONE_HOUR};
use crate::worker::ModelUsage;
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

pub fn reply_entries(reply: &str) -> Vec<(super::Role, String)> {
    vec![(super::Role::Assistant, reply.to_string())]
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CallKind {
    Turn {
        trigger: Trigger,
    },
    Nap,
    Worker {
        worker_id: String,
        turn_id: Option<u64>,
    },
    Ping,
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

const MAIN_HARNESS: &str = "claude";

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
        tools: Vec::new(),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_turn: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset: Option<ResetReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_source: Option<CostSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_5m: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<u64>,
    #[serde(flatten)]
    pub view: Option<ViewStats>,
}

impl UsageRecord {
    pub fn with_view(self, view: ViewStats) -> UsageRecord {
        UsageRecord {
            view: Some(view),
            ..self
        }
    }
}

pub fn usage_record(
    kind: CallKind,
    at: String,
    costs: &IterationCosts,
    metadata: &IterationMetadata,
) -> Option<UsageRecord> {
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
        process_turn: None,
        request_at: None,
        reset: None,
        run: None,
        store: None,
        harness: Some(MAIN_HARNESS.to_string()),
        model: metadata.model.clone(),
        session_id: metadata.session_id.clone(),
        cost_source: costs.cost_usd.map(|_| CostSource::Harness),
        reasoning_tokens: None,
        cache_write_5m: None,
        cache_write_1h: None,
        view: None,
    })
}

pub fn worker_usage_record(
    kind: CallKind,
    at: String,
    session_id: Option<String>,
    harness: &str,
    row: &ModelUsage,
) -> UsageRecord {
    let usage = &row.usage;
    let written = |tier| {
        usage
            .cache_writes
            .iter()
            .find(|write| write.tier == tier)
            .map(|write| write.tokens)
    };
    UsageRecord {
        at,
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
        harness: Some(harness.to_string()),
        model: Some(row.model.clone()),
        session_id,
        cost_source: None,
        reasoning_tokens: usage.reasoning,
        cache_write_5m: written(FIVE_MIN),
        cache_write_1h: written(ONE_HOUR),
        view: None,
    }
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
            terminal_reason: None,
        }
    }

    #[test]
    fn worker_rows_carry_every_collected_field() {
        use crate::wire::usage::{CacheWrite, Usage};
        let row = ModelUsage {
            model: "m".into(),
            usage: Usage {
                input_uncached: 1,
                cache_read: 2,
                cache_writes: vec![
                    CacheWrite {
                        tier: FIVE_MIN,
                        tokens: 3,
                    },
                    CacheWrite {
                        tier: ONE_HOUR,
                        tokens: 4,
                    },
                ],
                output: 5,
                reasoning: Some(6),
                other: Vec::new(),
            },
        };
        let kind = CallKind::Worker {
            worker_id: "w1".into(),
            turn_id: Some(2),
        };
        let value: Value = serde_json::from_str(
            &encode_usage(&worker_usage_record(
                kind,
                "t".into(),
                Some("s".into()),
                "claude",
                &row,
            ))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(value["kind"], "worker");
        assert_eq!(value["worker_id"], "w1");
        assert_eq!(value["turn_id"], 2);
        assert_eq!(value["cache_creation_input_tokens"], 7);
        assert_eq!(value["cache_write_5m"], 3);
        assert_eq!(value["cache_write_1h"], 4);
        assert_eq!(value["reasoning_tokens"], 6);
        assert_eq!(value["harness"], "claude");
        assert_eq!(value["model"], "m");
        assert_eq!(value["session_id"], "s");
        assert!(value["cost_usd"].is_null());
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
            reply_entries("r"),
            vec![(crate::chat_memory::Role::Assistant, "r".into())]
        );
    }

    #[test]
    fn usage_record_maps_all_fields() {
        let record = usage_record(
            CallKind::Nap,
            "t".into(),
            &costs(Some(7)),
            &IterationMetadata::new(),
        )
        .unwrap();
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
            &IterationCosts::new(),
            &IterationMetadata::new()
        )
        .is_none());
        let record = usage_record(
            CallKind::Turn {
                trigger: Trigger::User,
            },
            "t".into(),
            &costs(None),
            &IterationMetadata::new(),
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
        let value =
            line(usage_record(kind.clone(), "t".into(), &with, &IterationMetadata::new()).unwrap());
        assert_eq!(value["context_tokens"], 42);
        let value =
            line(usage_record(kind, "t".into(), &costs(None), &IterationMetadata::new()).unwrap());
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
            &IterationMetadata::new(),
        )
        .unwrap();
        let line = encode_usage(&record).unwrap();
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["kind"], "turn");
        assert_eq!(value["trigger"], "user");
    }

    #[test]
    fn encode_usage_includes_store_only_when_set() {
        let mut record = usage_record(
            CallKind::Nap,
            "t".into(),
            &costs(None),
            &IterationMetadata::new(),
        )
        .unwrap();
        let bare: Value = serde_json::from_str(&encode_usage(&record).unwrap()).unwrap();
        assert!(bare.get("store").is_none());
        record.store = Some("/chat store".into());
        let set: Value = serde_json::from_str(&encode_usage(&record).unwrap()).unwrap();
        assert_eq!(set["store"], "/chat store");
    }

    #[test]
    fn worker_turn_record_has_trigger() {
        let kind = CallKind::Turn {
            trigger: Trigger::Worker,
        };
        let record =
            usage_record(kind, "t".into(), &costs(None), &IterationMetadata::new()).unwrap();
        let value: Value = serde_json::from_str(&encode_usage(&record).unwrap()).unwrap();
        assert_eq!(
            (&value["kind"], &value["trigger"]),
            (&"turn".into(), &"worker".into())
        );
    }

    #[test]
    fn nap_record_has_no_trigger() {
        let record = usage_record(
            CallKind::Nap,
            "t".into(),
            &costs(None),
            &IterationMetadata::new(),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&encode_usage(&record).unwrap()).unwrap();
        assert_eq!(value["kind"], "nap");
        assert!(value.get("trigger").is_none());
    }

    #[test]
    fn ping_record_carries_model_session_and_harness() {
        let value = line(
            usage_record(
                CallKind::Ping,
                "t".into(),
                &costs(None),
                &seen(Some("m"), Some("s")),
            )
            .unwrap(),
        );
        assert_eq!(value["kind"], "ping");
        assert_eq!(
            (
                &value["harness"],
                &value["model"],
                &value["session_id"],
                &value["cost_source"]
            ),
            (
                &"claude".into(),
                &"m".into(),
                &"s".into(),
                &"harness".into()
            )
        );
    }

    #[test]
    fn ping_record_has_the_ping_kind_run_and_request_time() {
        let mut record = usage_record(
            CallKind::Ping,
            "t".into(),
            &costs(None),
            &IterationMetadata::new(),
        )
        .unwrap();
        record.run = Some("r".into());
        record.request_at = Some("q".into());
        let value: Value = serde_json::from_str(&encode_usage(&record).unwrap()).unwrap();
        assert_eq!(value["kind"], "ping");
        assert_eq!(
            (&value["run"], &value["request_at"]),
            (&"r".into(), &"q".into())
        );
        assert!(value.get("trigger").is_none() && value.get("process_turn").is_none());
    }

    #[test]
    fn turn_record_carries_view_stats_and_nap_does_not() {
        let lines = vec!["#0-1 a".to_string(), "#2 b".to_string()];
        let stats = view_stats(&lines);
        assert_eq!((stats.view_bytes, stats.view_lines), (11, 2));
        let kind = CallKind::Turn {
            trigger: Trigger::User,
        };
        let record = usage_record(kind, "t".into(), &costs(None), &IterationMetadata::new())
            .unwrap()
            .with_view(stats.clone());
        let value: Value = serde_json::from_str(&encode_usage(&record).unwrap()).unwrap();
        assert_eq!(value["view_bytes"], 11);
        assert_eq!(value["view_lines"], 2);
        assert_eq!(value["view_head_hash"], stats.view_head_hash.as_str());
        assert_eq!(value["view_hash"].as_str().map(str::len), Some(16));
        let nap = usage_record(
            CallKind::Nap,
            "t".into(),
            &costs(None),
            &IterationMetadata::new(),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&encode_usage(&nap).unwrap()).unwrap();
        assert!(value.get("view_bytes").is_none());
    }

    #[test]
    fn thread_fields_are_serialized_only_when_present() {
        let mut record = usage_record(
            CallKind::Turn {
                trigger: Trigger::User,
            },
            "t".into(),
            &costs(None),
            &IterationMetadata::new(),
        )
        .unwrap();
        let value = line(
            usage_record(
                CallKind::Nap,
                "t".into(),
                &costs(None),
                &IterationMetadata::new(),
            )
            .unwrap(),
        );
        for key in ["process_turn", "request_at", "reset", "run"] {
            assert!(value.get(key).is_none(), "{key}");
        }
        record.process_turn = Some(3);
        record.request_at = Some("2026-01-02T03:04:05+00:00".into());
        record.reset = Some(ResetReason::Restart);
        record.run = Some("7-t".into());
        let value = line(record);
        assert_eq!(value["run"], "7-t");
        assert_eq!(value["process_turn"], 3);
        assert_eq!(value["request_at"], "2026-01-02T03:04:05+00:00");
        assert_eq!(value["reset"], "restart");
    }

    fn seen(model: Option<&str>, session: Option<&str>) -> IterationMetadata {
        IterationMetadata {
            model: model.map(String::from),
            session_id: session.map(String::from),
            ..IterationMetadata::new()
        }
    }

    #[test]
    fn identity_fields_serialize_only_when_present() {
        let bare =
            line(usage_record(CallKind::Nap, "t".into(), &costs(None), &seen(None, None)).unwrap());
        assert_eq!(bare["harness"], "claude");
        assert_eq!(bare["cost_source"], "harness");
        assert!(bare.get("model").is_none() && bare.get("session_id").is_none());
        let full = line(
            usage_record(
                CallKind::Nap,
                "t".into(),
                &costs(None),
                &seen(Some("m"), Some("s")),
            )
            .unwrap(),
        );
        assert_eq!(
            (&full["model"], &full["session_id"]),
            (&"m".into(), &"s".into())
        );
    }

    #[test]
    fn cost_source_is_absent_without_cost() {
        let mut no_cost = costs(None);
        no_cost.cost_usd = None;
        let value =
            line(usage_record(CallKind::Nap, "t".into(), &no_cost, &seen(None, None)).unwrap());
        assert!(value.get("cost_source").is_none());
        assert!(value["cost_usd"].is_null());
    }

    #[test]
    fn carry_metadata_keeps_launch_values_until_relaunch() {
        let launch = carry_metadata(&IterationMetadata::new(), true, &seen(Some("m"), Some("s")));
        let later = carry_metadata(&launch, false, &seen(None, None));
        assert_eq!(
            (later.model.as_deref(), later.session_id.as_deref()),
            (Some("m"), Some("s"))
        );
        let relaunch = carry_metadata(&later, true, &seen(None, None));
        assert!(relaunch.is_empty());
    }

    #[test]
    fn old_lines_without_identity_fields_still_parse() {
        let old = r#"{"at":"t","kind":"turn","trigger":"user","input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":2,"cost_usd":0.5,"duration_ms":4,"run":"r","process_turn":1}"#;
        let new = r#"{"at":"t","kind":"turn","trigger":"user","input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":2,"cost_usd":0.5,"duration_ms":4,"run":"r","process_turn":2,"harness":"claude","model":"m","session_id":"s","cost_source":"harness"}"#;
        let text = format!("{old}\n{new}\n");
        assert_eq!(crate::chat_thread::session_rows(&text, "r").len(), 2);
        assert!(
            crate::chat_thread::thread_usage(&crate::chat_thread::session_rows(&text, "r"))
                .is_some()
        );
    }

    fn line(record: UsageRecord) -> Value {
        serde_json::from_str(&encode_usage(&record).unwrap()).unwrap()
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
