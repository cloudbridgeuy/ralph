use crate::stream::IterationCosts;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    User,
    Worker,
}

impl Trigger {
    pub fn role(self) -> super::Role {
        match self {
            Trigger::User => super::Role::User,
            Trigger::Worker => super::Role::Worker,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CallKind {
    Turn { trigger: Trigger },
    Nap,
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
        }
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
    fn trigger_maps_to_role() {
        assert_eq!(Trigger::User.role(), super::super::Role::User);
        assert_eq!(Trigger::Worker.role(), super::super::Role::Worker);
    }
}
