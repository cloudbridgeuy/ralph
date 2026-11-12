use crate::chat::memory::usage_path;
use crate::ledger;
use crate::warn::warn_if_err;
use ralph_core::worker::{print_usage_record, ModelUsage};
use std::io;
use std::path::Path;

pub struct CursorTurn<'a> {
    pub worker: &'a str,
    pub chat: Option<&'a str>,
    pub store: Option<&'a Path>,
    pub turn_id: Option<u64>,
}

pub fn record_cursor_usage(turn: &CursorTurn, usage: &ModelUsage) {
    warn_if_err(
        append(turn, usage, &usage_path()),
        "Failed to record cursor worker usage",
    );
}

fn append(turn: &CursorTurn, usage: &ModelUsage, usage_file: &Path) -> io::Result<()> {
    let at = chrono::Utc::now().to_rfc3339();
    let row = print_usage_record(
        turn.worker,
        turn.turn_id,
        at,
        turn.chat.map(str::to_string),
        usage,
    )
    .with_store(turn.store.map(|store| store.to_string_lossy().into_owned()))
    .with_ticket(ledger::worker_ticket(turn.store, turn.worker));
    ledger::record(usage_file, &row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::worker::map_print_usage;
    use serde_json::{json, Value};

    #[test]
    fn a_result_appends_one_cursor_worker_row_with_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("usage.jsonl");
        let counters = json!({"inputTokens":8680,"outputTokens":36,"cacheReadTokens":6016,"cacheWriteTokens":0});
        let usage = map_print_usage(&counters, "composer-2").unwrap();
        let turn = CursorTurn {
            worker: "w1",
            chat: Some("chat-1"),
            store: Some(dir.path()),
            turn_id: None,
        };
        append(&turn, &usage, &file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), 1);
        let row: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(row["harness"], "cursor");
        assert_eq!(row["input_tokens"], 8680);
        assert_eq!(row["session_id"], "chat-1");
        assert_eq!(row["store"], dir.path().to_string_lossy().as_ref());
    }

    #[test]
    fn a_cursor_row_carries_the_ticket_of_its_spawn_record() {
        let dir = tempfile::tempdir().unwrap();
        crate::ledger::fixtures::spawn_record(dir.path(), "w1", "GUZ-9");
        let file = dir.path().join("usage.jsonl");
        let counters =
            json!({"inputTokens":8,"outputTokens":3,"cacheReadTokens":0,"cacheWriteTokens":0});
        let usage = map_print_usage(&counters, "composer-2").unwrap();
        let turn = CursorTurn {
            worker: "w1",
            chat: None,
            store: Some(dir.path()),
            turn_id: None,
        };
        append(&turn, &usage, &file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let row: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(row["ticket"], "GUZ-9");
    }
}
