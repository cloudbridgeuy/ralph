use crate::ledger;
use agent_client_protocol::schema::v1::{PromptResponse, Usage};
use ralph_core::chat_memory::{worker_usage_record, CallSite};
use ralph_core::worker::{agy_turn_usage, AgyUsage, ModelUsage};
use std::io;
use std::path::Path;

pub(crate) struct Target<'a> {
    pub worker: &'a str,
    pub session: &'a str,
    pub model: &'a str,
    pub store: Option<&'a Path>,
    pub turn_id: Option<u64>,
}

pub(crate) fn record(
    path: &Path,
    target: &Target<'_>,
    response: &PromptResponse,
) -> io::Result<()> {
    let kind = CallSite::Worker {
        worker_id: target.worker.to_string(),
        turn_id: target.turn_id,
    };
    record_as(path, target, response, kind)
}

pub(crate) fn record_verify(
    path: &Path,
    target: &Target<'_>,
    response: &PromptResponse,
) -> io::Result<()> {
    let kind = CallSite::Verify {
        worker_id: target.worker.to_string(),
    };
    record_as(path, target, response, kind)
}

pub(crate) fn raw_usage(raw: &Usage) -> AgyUsage {
    AgyUsage {
        total: raw.total_tokens,
        input: raw.input_tokens,
        output: raw.output_tokens,
        read: raw.cached_read_tokens,
        write: raw.cached_write_tokens,
        thought: raw.thought_tokens,
    }
}

fn record_as(
    path: &Path,
    target: &Target<'_>,
    response: &PromptResponse,
    kind: CallSite,
) -> io::Result<()> {
    let Some(raw) = &response.usage else {
        return Ok(());
    };
    let Some(usage) = agy_turn_usage(&raw_usage(raw))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
    else {
        return Ok(());
    };
    let store = target.store.map(Path::canonicalize).transpose()?;
    let usage = ModelUsage {
        model: target.model.to_string(),
        usage,
        long_prompt: false,
    };
    let row = worker_usage_record(
        kind,
        chrono::Utc::now().to_rfc3339(),
        Some(target.session.to_string()),
        "agy",
        &usage,
    )
    .with_store(
        store
            .as_ref()
            .map(|store| store.to_string_lossy().into_owned()),
    )
    .with_ticket(ledger::worker_ticket(store.as_deref(), target.worker));
    ledger::record(path, &row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn response(total: u64) -> Result<PromptResponse, serde_json::Error> {
        serde_json::from_value(
            json!({"stopReason":"end_turn","usage":{"inputTokens":100,"outputTokens":20,"cachedReadTokens":30,"cachedWriteTokens":10,"thoughtTokens":5,"totalTokens":total}}),
        )
    }

    #[test]
    fn agy_reconciles_each_reported_total_and_writes_turns(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("usage.jsonl");
        crate::ledger::fixtures::spawn_record(dir.path(), "a", "GUZ-9");
        let target = Target {
            worker: "a",
            session: "s",
            model: "claude-opus-5-5",
            store: Some(dir.path()),
            turn_id: Some(7),
        };
        for total in [120, 125, 160, 165] {
            record(&path, &target, &response(total)?)?;
        }
        let rows: Vec<Value> = std::fs::read_to_string(path)?
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        assert_eq!(rows.len(), 4);
        for (row, (input, output)) in rows.iter().zip([(60, 20), (60, 25), (100, 20), (100, 25)]) {
            assert_eq!(row["input_tokens"], input);
            assert_eq!(row["output_tokens"], output);
            assert_eq!(row["cache_read_input_tokens"], 30);
            assert_eq!(row["cache_creation_input_tokens"], 10);
            assert_eq!(row["reasoning_tokens"], 5);
            assert_eq!(row["worker_id"], "a");
            assert_eq!(row["session_id"], "s");
            assert_eq!(row["model"], "claude-opus-5-5");
            assert_eq!(row["turn_id"], 7);
            assert_eq!(row["ticket"], "GUZ-9");
            assert_eq!(
                row["store"],
                dir.path().canonicalize()?.to_string_lossy().as_ref()
            );
            assert!(row["cost_usd"].is_null() && row.get("cost_source").is_none());
        }
        Ok(())
    }

    #[test]
    fn agy_rejects_ambiguous_and_inconsistent_usage_visibly(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = Target {
            worker: "a",
            session: "s",
            model: "m",
            store: None,
            turn_id: None,
        };
        let path = dir.path().join("usage");
        let ambiguous: PromptResponse = serde_json::from_value(
            json!({"stopReason":"end_turn","usage":{"inputTokens":100,"outputTokens":20,"cachedReadTokens":10,"thoughtTokens":10,"totalTokens":130}}),
        )?;
        assert!(record(&path, &target, &ambiguous).is_err());
        assert!(record(&path, &target, &response(999)?).is_err());
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn agy_missing_and_empty_usage_do_not_write() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = Target {
            worker: "a",
            session: "s",
            model: "m",
            store: None,
            turn_id: None,
        };
        let path = dir.path().join("usage");
        for value in [
            json!({"stopReason":"end_turn"}),
            json!({"stopReason":"end_turn","usage":{"inputTokens":0,"outputTokens":0,"totalTokens":0}}),
        ] {
            record(&path, &target, &serde_json::from_value(value)?)?;
        }
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn agy_optional_counters_and_sessions_remain_distinct() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("usage");
        let response: PromptResponse = serde_json::from_value(
            json!({"stopReason":"end_turn","usage":{"inputTokens":8,"outputTokens":2,"totalTokens":10}}),
        )?;
        for session in ["s1", "s2"] {
            let target = Target {
                worker: "a",
                session,
                model: "m",
                store: None,
                turn_id: None,
            };
            record(&path, &target, &response)?;
        }
        let rows: Vec<Value> = std::fs::read_to_string(path)?
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (
                rows[0]["session_id"].as_str(),
                rows[1]["session_id"].as_str()
            ),
            (Some("s1"), Some("s2"))
        );
        assert!(rows[0].get("reasoning_tokens").is_none());
        Ok(())
    }
    #[cfg(unix)]
    #[test]
    fn agy_ledger_store_resolves_alias() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = dir.path().join("store");
        let alias = dir.path().join("alias");
        std::fs::create_dir(&store)?;
        std::os::unix::fs::symlink(&store, &alias)?;
        let path = dir.path().join("usage");
        let target = Target {
            worker: "a",
            session: "s",
            model: "m",
            store: Some(&alias),
            turn_id: None,
        };
        record(&path, &target, &response(120)?)?;
        let row: Value = serde_json::from_str(std::fs::read_to_string(path)?.trim())?;
        assert_eq!(
            row["store"],
            store.canonicalize()?.to_string_lossy().as_ref()
        );
        Ok(())
    }
}
