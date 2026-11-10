use crate::chat::memory::usage_path;
use crate::ledger;
use crate::warn::warn_if_err;
use ralph_core::chat_memory::{worker_usage_record, CallSite};
use ralph_core::worker::{decode_thread_parent, decode_usage_event, ModelUsage, TurnUsage};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const HARNESS: &str = "codex";

#[derive(Default)]
struct Bound {
    thread: String,
    turn: TurnUsage,
    parents: BTreeMap<String, String>,
    active: bool,
}

impl Bound {
    fn descends_from_bound(&self, params: &Value) -> bool {
        let mut at = params["threadId"].as_str();
        for _ in 0..=self.parents.len() {
            match at {
                Some(thread) if thread == self.thread => return true,
                Some(thread) => at = self.parents.get(thread).map(String::as_str),
                None => return false,
            }
        }
        false
    }
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
            ..Bound::default()
        };
    }

    pub fn observe(&self, method: &str, params: &Value, in_turn: bool) {
        let mut bound = self.bound.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((thread, parent)) = decode_thread_parent(method, params) {
            bound.parents.insert(thread, parent);
        }
        if let Some(event) = decode_usage_event(method, params) {
            let in_turn = in_turn || (bound.active && bound.descends_from_bound(params));
            bound.turn.observe(event, in_turn);
        }
    }

    pub fn start_turn(&self) {
        let mut bound = self.bound.lock().unwrap_or_else(PoisonError::into_inner);
        bound.turn.start_turn();
        bound.active = true;
    }

    pub fn flush(&self, turn_id: Option<u64>) {
        let (thread, rows) = {
            let mut bound = self.bound.lock().unwrap_or_else(PoisonError::into_inner);
            bound.active = false;
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
            let kind = CallSite::Worker {
                worker_id: self.worker.clone(),
                turn_id,
            };
            let usage =
                worker_usage_record(kind, at.clone(), Some(thread.to_string()), HARNESS, row)
                    .with_subagent_usage()
                    .with_store(
                        self.store
                            .as_ref()
                            .map(|store| store.to_string_lossy().into_owned()),
                    );
            ledger::record(usage_file, &usage)?;
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
        assert_eq!(row["subagent_usage"], true);
        assert!(usage.bound.lock().unwrap().turn.take().is_empty());
    }

    fn usage_params(thread: &str, input: u64, total: u64) -> Value {
        serde_json::json!({"threadId": thread, "turnId": "child-turn", "tokenUsage": {"last": {"inputTokens": input}, "total": {"inputTokens": total}}})
    }

    fn started(thread: &str, parent: &str) -> Value {
        serde_json::json!({"thread": {"id": thread, "parentThreadId": parent}})
    }

    fn counted(usage: &CodexUsage) -> Vec<u64> {
        let rows = usage.bound.lock().unwrap().turn.take();
        rows.iter().map(|row| row.usage.input_uncached).collect()
    }

    #[test]
    fn child_thread_usage_counts_in_the_active_turn() {
        let usage = CodexUsage::new("w1", None);
        usage.bind("parent", Some("gpt".into()));
        usage.start_turn();
        usage.observe("thread/started", &started("child", "parent"), false);
        usage.observe("thread/started", &started("grand", "child"), false);
        usage.observe(
            "thread/tokenUsage/updated",
            &usage_params("parent", 100, 100),
            true,
        );
        usage.observe(
            "thread/tokenUsage/updated",
            &usage_params("child", 5, 5),
            false,
        );
        usage.observe(
            "thread/tokenUsage/updated",
            &usage_params("grand", 3, 3),
            false,
        );
        usage.observe(
            "thread/tokenUsage/updated",
            &usage_params("parent", 10, 110),
            true,
        );
        assert_eq!(counted(&usage), [118]);
    }

    #[test]
    fn unrelated_thread_usage_and_child_usage_between_turns_are_not_counted() {
        let usage = CodexUsage::new("w1", None);
        usage.bind("parent", Some("gpt".into()));
        usage.observe("thread/started", &started("child", "parent"), false);
        usage.observe(
            "thread/tokenUsage/updated",
            &usage_params("child", 5, 5),
            false,
        );
        usage.start_turn();
        usage.observe(
            "thread/tokenUsage/updated",
            &usage_params("other", 9, 9),
            false,
        );
        usage.observe(
            "thread/tokenUsage/updated",
            &usage_params("child", 2, 7),
            false,
        );
        assert_eq!(counted(&usage), [2]);
    }
}
