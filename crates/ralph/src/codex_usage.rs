use crate::chat::memory::{append_usage, usage_path};
use crate::warn::warn_if_err;
use ralph_core::chat_memory::{worker_usage_record, CallKind};
use ralph_core::worker::{decode_usage_event, ModelUsage, TurnUsage};
use serde_json::Value;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const HARNESS: &str = "codex";

#[derive(Default)]
struct Bound {
    thread: String,
    turn: TurnUsage,
}

pub struct CodexUsage {
    worker: String,
    store: Option<PathBuf>,
    bound: Mutex<Bound>,
}

impl CodexUsage {
    pub fn new(worker: &str, store: Option<&Path>) -> Self {
        Self {
            worker: worker.to_string(),
            store: store.map(Path::to_path_buf),
            bound: Mutex::default(),
        }
    }

    pub fn bind(&self, thread: &str, model: Option<String>) {
        *self.bound.lock().unwrap_or_else(PoisonError::into_inner) = Bound {
            thread: thread.to_string(),
            turn: TurnUsage::new(model),
        };
    }

    pub fn observe(&self, method: &str, params: &Value, in_turn: bool) {
        if let Some(event) = decode_usage_event(method, params) {
            let mut bound = self.bound.lock().unwrap_or_else(PoisonError::into_inner);
            bound.turn.observe(event, in_turn);
        }
    }

    pub fn start_turn(&self) {
        let mut bound = self.bound.lock().unwrap_or_else(PoisonError::into_inner);
        bound.turn.start_turn();
    }

    pub fn flush(&self, turn_id: Option<u64>) {
        let (thread, rows) = {
            let mut bound = self.bound.lock().unwrap_or_else(PoisonError::into_inner);
            (bound.thread.clone(), bound.turn.take())
        };
        warn_if_err(
            self.append(&thread, &rows, turn_id, &usage_path()),
            "Failed to record codex worker usage",
        );
    }

    fn append(
        &self,
        thread: &str,
        rows: &[ModelUsage],
        turn_id: Option<u64>,
        usage_file: &Path,
    ) -> io::Result<()> {
        let at = chrono::Utc::now().to_rfc3339();
        for row in rows {
            let kind = CallKind::Worker {
                worker_id: self.worker.clone(),
                turn_id,
            };
            let mut usage =
                worker_usage_record(kind, at.clone(), Some(thread.to_string()), HARNESS, row);
            usage.store = self
                .store
                .as_ref()
                .map(|store| store.to_string_lossy().into_owned());
            append_usage(usage_file, &usage)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_appends_codex_worker_rows_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("usage.jsonl");
        let usage = CodexUsage::new("w1", Some(dir.path()));
        usage.bind("th1", Some("gpt".into()));
        let params = serde_json::json!({"turnId": "u", "tokenUsage": {"last": {"inputTokens": 9, "cachedInputTokens": 4, "outputTokens": 2}, "total": {"inputTokens": 9, "cachedInputTokens": 4, "outputTokens": 2}}});
        usage.observe("thread/tokenUsage/updated", &params, true);
        let rows = usage.bound.lock().unwrap().turn.take();
        usage.append("th1", &rows, Some(3), &file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let row: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(row["kind"], "worker");
        assert_eq!(row["harness"], "codex");
        assert_eq!(row["session_id"], "th1");
        assert_eq!(row["model"], "gpt");
        assert_eq!(row["input_tokens"], 5);
        assert_eq!(row["turn_id"], 3);
        assert!(usage.bound.lock().unwrap().turn.take().is_empty());
    }
}
