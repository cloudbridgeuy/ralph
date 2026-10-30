use super::WORKERS_DIR;
use crate::wire::provider::anthropic::map_usage;
use crate::wire::usage::Usage;
use crate::worker_costs::long_prompt_call;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelUsage {
    pub model: String,
    pub usage: Usage,
    pub long_prompt: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TranscriptUsage {
    pub rows: Vec<ModelUsage>,
    pub recorded: Vec<String>,
}

pub fn usage_seen_path(store: &Path, id: &str) -> PathBuf {
    store.join(WORKERS_DIR).join(format!("{id}.usage-seen"))
}

pub fn subagents_dir(transcript: &Path) -> PathBuf {
    transcript.with_extension("").join("subagents")
}

const SYNTHETIC_MODEL: &str = "<synthetic>";

fn is_empty(usage: &Usage) -> bool {
    usage.input_uncached == 0
        && usage.cache_read == 0
        && usage.output == 0
        && usage.reasoning.unwrap_or(0) == 0
        && usage.cache_writes.iter().all(|write| write.tokens == 0)
        && usage.other.iter().all(|(_, tokens)| *tokens == 0)
}

fn add(total: &mut Usage, part: Usage) {
    total.input_uncached += part.input_uncached;
    total.cache_read += part.cache_read;
    total.output += part.output;
    total.reasoning = match (total.reasoning, part.reasoning) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    };
    for write in part.cache_writes {
        match total.cache_writes.iter_mut().find(|w| w.tier == write.tier) {
            Some(existing) => existing.tokens += write.tokens,
            None => total.cache_writes.push(write),
        }
    }
}

pub fn transcript_usage(lines: &str, seen: &HashSet<String>) -> TranscriptUsage {
    let mut latest: BTreeMap<String, Value> = BTreeMap::new();
    for value in lines
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| value.get("type").and_then(Value::as_str) == Some("assistant"))
    {
        let message = value.get("message");
        if let Some(id) = message.and_then(|m| m.get("id")).and_then(Value::as_str) {
            latest.insert(id.to_string(), message.cloned().unwrap_or(Value::Null));
        }
    }
    let mut by_model: BTreeMap<(String, bool), Usage> = BTreeMap::new();
    let mut recorded = Vec::new();
    for (id, message) in latest {
        let finished = message
            .get("stop_reason")
            .is_some_and(|reason| !reason.is_null());
        let model = message.get("model").and_then(Value::as_str);
        let usage = message.get("usage").filter(|usage| usage.is_object());
        let (true, false, Some(model), Some(usage)) = (finished, seen.contains(&id), model, usage)
        else {
            continue;
        };
        let part = map_usage(usage);
        recorded.push(id);
        if model == SYNTHETIC_MODEL || is_empty(&part) {
            continue;
        }
        let key = (model.to_string(), long_prompt_call(model, &part));
        match by_model.get_mut(&key) {
            Some(total) => add(total, part),
            None => {
                by_model.insert(key, part);
            }
        }
    }
    TranscriptUsage {
        rows: by_model
            .into_iter()
            .map(|((model, long_prompt), usage)| ModelUsage {
                model,
                usage,
                long_prompt,
            })
            .collect(),
        recorded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::usage::{CacheWrite, FIVE_MIN, ONE_HOUR};

    fn line(id: &str, model: &str, stop: Option<&str>, output: u64) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {
                "id": id,
                "model": model,
                "stop_reason": stop,
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": output,
                    "cache_read_input_tokens": 100,
                    "cache_creation_input_tokens": 7,
                    "cache_creation": {
                        "ephemeral_5m_input_tokens": 3,
                        "ephemeral_1h_input_tokens": 4
                    },
                    "output_tokens_details": {"thinking_tokens": 2}
                }
            }
        })
        .to_string()
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    fn zero_line(id: &str, model: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {
                "id": id,
                "model": model,
                "stop_reason": "stop_sequence",
                "usage": {"input_tokens": 0, "output_tokens": 0}
            }
        })
        .to_string()
    }

    #[test]
    fn usage_is_empty_only_when_every_count_is_zero() {
        let zero = Usage {
            input_uncached: 0,
            cache_read: 0,
            cache_writes: vec![CacheWrite {
                tier: FIVE_MIN,
                tokens: 0,
            }],
            output: 0,
            reasoning: Some(0),
            other: vec![("x".to_string(), 0)],
        };
        assert!(is_empty(&zero));
        let mut cases = vec![zero.clone(); 6];
        cases[0].input_uncached = 1;
        cases[1].cache_read = 1;
        cases[2].output = 1;
        cases[3].reasoning = Some(1);
        cases[4].cache_writes[0].tokens = 1;
        cases[5].other[0].1 = 1;
        assert!(cases.iter().all(|usage| !is_empty(usage)));
    }

    #[test]
    fn synthetic_messages_give_no_row() {
        let found = transcript_usage(&zero_line("m1", "<synthetic>"), &none());
        assert!(found.rows.is_empty());
        assert_eq!(found.recorded, ["m1"]);
    }

    #[test]
    fn synthetic_messages_with_tokens_give_no_row() {
        let found = transcript_usage(&line("m1", "<synthetic>", Some("end_turn"), 5), &none());
        assert!(found.rows.is_empty());
        assert_eq!(found.recorded, ["m1"]);
    }

    #[test]
    fn all_zero_usage_gives_no_row() {
        let found = transcript_usage(&zero_line("m1", "opus"), &none());
        assert!(found.rows.is_empty());
        assert_eq!(found.recorded, ["m1"]);
    }

    #[test]
    fn zero_messages_do_not_hide_real_ones() {
        let text = [
            zero_line("m1", "opus"),
            line("m2", "opus", Some("end_turn"), 5),
        ]
        .join("\n");
        let found = transcript_usage(&text, &none());
        assert_eq!(found.recorded, ["m1", "m2"]);
        assert_eq!(found.rows[0].usage.output, 5);
    }

    #[test]
    fn split_message_lines_count_once_from_the_last_line() {
        let text = [
            line("m1", "opus", None, 1),
            line("m1", "opus", Some("end_turn"), 50),
        ]
        .join("\n");
        let found = transcript_usage(&text, &none());
        assert_eq!(found.recorded, ["m1"]);
        assert_eq!(found.rows.len(), 1);
        let usage = &found.rows[0].usage;
        assert_eq!(usage.output, 50);
        assert_eq!(usage.input_uncached, 10);
        assert_eq!(usage.reasoning, Some(2));
        assert_eq!(
            usage.cache_writes,
            [
                CacheWrite {
                    tier: FIVE_MIN,
                    tokens: 3
                },
                CacheWrite {
                    tier: ONE_HOUR,
                    tokens: 4
                }
            ]
        );
    }

    #[test]
    fn seen_ids_are_skipped() {
        let text = [
            line("m1", "opus", Some("end_turn"), 5),
            line("m2", "opus", Some("end_turn"), 6),
        ]
        .join("\n");
        let seen = HashSet::from(["m1".to_string()]);
        let found = transcript_usage(&text, &seen);
        assert_eq!(found.recorded, ["m2"]);
        assert_eq!(found.rows[0].usage.output, 6);
    }

    #[test]
    fn each_model_gets_its_own_row_with_summed_tokens() {
        let text = [
            line("m1", "opus", Some("end_turn"), 5),
            line("m2", "haiku", Some("end_turn"), 6),
            line("m3", "opus", Some("tool_use"), 7),
        ]
        .join("\n");
        let found = transcript_usage(&text, &none());
        let models: Vec<_> = found.rows.iter().map(|row| row.model.as_str()).collect();
        assert_eq!(models, ["haiku", "opus"]);
        assert_eq!(found.rows[1].usage.output, 12);
        assert_eq!(found.rows[1].usage.cache_writes[0].tokens, 6);
    }

    fn call(id: &str, model: &str, input: u64) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {
                "id": id,
                "model": model,
                "stop_reason": "end_turn",
                "usage": {"input_tokens": input, "output_tokens": 1}
            }
        })
        .to_string()
    }

    #[test]
    fn many_short_calls_stay_one_short_row() {
        let text: Vec<String> = (0..10)
            .map(|n| call(&format!("m{n}"), "claude-haiku-5-5", 30_000))
            .collect();
        let found = transcript_usage(&text.join("\n"), &none());
        assert_eq!(found.rows.len(), 1);
        assert_eq!(found.rows[0].usage.input_uncached, 300_000);
        assert!(!found.rows[0].long_prompt);
    }

    #[test]
    fn a_long_call_gets_its_own_row() {
        let text = [
            call("m1", "claude-haiku-5-5", 150_000),
            call("m2", "claude-haiku-5-5", 10_000),
        ]
        .join("\n");
        let found = transcript_usage(&text, &none());
        let rows: Vec<_> = found
            .rows
            .iter()
            .map(|row| (row.long_prompt, row.usage.input_uncached))
            .collect();
        assert_eq!(rows, [(false, 10_000), (true, 150_000)]);
    }

    #[test]
    fn in_flight_messages_wait_for_a_later_stop() {
        let found = transcript_usage(&line("m1", "opus", None, 5), &none());
        assert_eq!(found, TranscriptUsage::default());
    }

    #[test]
    fn malformed_and_non_assistant_lines_are_ignored() {
        let text = format!(
            "not json\n{{\"type\":\"user\",\"message\":{{\"id\":\"u\"}}}}\n{{\"type\":\"assistant\"}}\n{}",
            line("m1", "opus", Some("end_turn"), 5)
        );
        assert_eq!(transcript_usage(&text, &none()).recorded, ["m1"]);
    }

    #[test]
    fn subagents_live_beside_the_transcript_without_its_extension() {
        assert_eq!(
            subagents_dir(Path::new("/p/abc.jsonl")),
            Path::new("/p/abc/subagents")
        );
    }
}
