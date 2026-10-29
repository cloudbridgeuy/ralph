use crate::chat::memory::{append_usage, usage_path};
use crate::warn::warn_if_err;
use ralph_core::chat_memory::CallKind;
use ralph_core::worker::{opencode_usage_record, OpencodeModelUsage};
use std::io;
use std::path::Path;

pub struct OpencodeTurn<'a> {
    pub worker: &'a str,
    pub session_id: &'a str,
    pub store: Option<&'a Path>,
    pub turn_id: Option<u64>,
}

pub fn record_opencode_usage(turn: &OpencodeTurn, rows: &[OpencodeModelUsage]) {
    warn_if_err(
        append(turn, rows, &usage_path()),
        "Failed to record opencode worker usage",
    );
}

fn append(turn: &OpencodeTurn, rows: &[OpencodeModelUsage], usage_file: &Path) -> io::Result<()> {
    let at = chrono::Utc::now().to_rfc3339();
    for spend in rows {
        let kind = CallKind::Worker {
            worker_id: turn.worker.to_string(),
            turn_id: turn.turn_id,
        };
        let mut usage = opencode_usage_record(kind, at.clone(), turn.session_id, spend);
        usage.store = turn.store.map(|store| store.to_string_lossy().into_owned());
        append_usage(usage_file, &usage)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::worker::{decode_opencode_usage, parse_event, OpencodeTurnUsage};
    use serde_json::Value;

    #[test]
    fn a_turn_appends_opencode_worker_rows_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("usage.jsonl");
        let line = r#"data: {"id":"e","type":"session.step.ended","data":{"sessionID":"ses_a","cost":0,"tokens":{"input":9,"output":2,"reasoning":1,"cache":{"read":4,"write":0}}}}"#;
        let mut usage = OpencodeTurnUsage::new("openai/gpt-6.1-sol");
        usage.observe(decode_opencode_usage(&parse_event(line).unwrap()).unwrap());
        let turn = OpencodeTurn {
            worker: "w1",
            session_id: "ses_a",
            store: Some(dir.path()),
            turn_id: Some(3),
        };
        append(&turn, &usage.take(), &file).unwrap();
        append(&turn, &usage.take(), &file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), 1);
        let row: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(row["harness"], "opencode");
        assert_eq!(row["model"], "openai/gpt-6.1-sol");
        assert_eq!(row["output_tokens"], 3);
        assert_eq!(row["turn_id"], 3);
        assert_eq!(row["store"], dir.path().to_string_lossy().as_ref());
    }
}
