use crate::chat::memory::usage_path;
use crate::ledger;
use crate::warn::warn_if_err;
use ralph_core::chat_memory::CallSite;
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
        append(turn, rows, worker_site(turn), &usage_path()),
        "Failed to record opencode worker usage",
    );
}

pub fn record_opencode_verify(turn: &OpencodeTurn, rows: &[OpencodeModelUsage], ledger: &Path) {
    let site = CallSite::Verify {
        worker_id: turn.worker.to_string(),
    };
    warn_if_err(
        append(turn, rows, site, ledger),
        "Failed to record opencode verify usage",
    );
}

fn worker_site(turn: &OpencodeTurn) -> CallSite {
    CallSite::Worker {
        worker_id: turn.worker.to_string(),
        turn_id: turn.turn_id,
    }
}

fn append(
    turn: &OpencodeTurn,
    rows: &[OpencodeModelUsage],
    site: CallSite,
    usage_file: &Path,
) -> io::Result<()> {
    let at = chrono::Utc::now().to_rfc3339();
    for spend in rows {
        let usage = opencode_usage_record(site.clone(), at.clone(), turn.session_id, spend)
            .with_store(turn.store.map(|store| store.to_string_lossy().into_owned()))
            .with_ticket(ledger::worker_ticket(turn.store, turn.worker));
        ledger::record(usage_file, &usage)?;
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
        append(&turn, &usage.take(), worker_site(&turn), &file).unwrap();
        append(&turn, &usage.take(), worker_site(&turn), &file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.lines().count(), 1);
        let row: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(row["harness"], "opencode");
        assert_eq!(row["model"], "openai/gpt-6.1-sol");
        assert_eq!(row["output_tokens"], 3);
        assert_eq!(row["turn_id"], 3);
        assert_eq!(row["store"], dir.path().to_string_lossy().as_ref());
    }

    #[test]
    fn an_opencode_row_carries_the_ticket_of_its_spawn_record() {
        let dir = tempfile::tempdir().unwrap();
        crate::ledger::fixtures::spawn_record(dir.path(), "w1", "GUZ-9");
        let file = dir.path().join("usage.jsonl");
        let line = r#"data: {"id":"e","type":"session.step.ended","data":{"sessionID":"ses_a","cost":0,"tokens":{"input":9,"output":2,"reasoning":1,"cache":{"read":4,"write":0}}}}"#;
        let mut usage = OpencodeTurnUsage::new("openai/gpt-6.1-sol");
        usage.observe(decode_opencode_usage(&parse_event(line).unwrap()).unwrap());
        let turn = OpencodeTurn {
            worker: "w1",
            session_id: "ses_a",
            store: Some(dir.path()),
            turn_id: None,
        };
        append(&turn, &usage.take(), worker_site(&turn), &file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let row: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(row["ticket"], "GUZ-9");
    }

    #[test]
    fn a_verify_record_appends_a_verify_row_that_keeps_the_reported_cost() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("usage.jsonl");
        let line = r#"data: {"id":"e","type":"session.step.ended","data":{"sessionID":"ses_a","cost":0.5,"tokens":{"input":9,"output":2,"reasoning":1,"cache":{"read":4,"write":0}}}}"#;
        let mut usage = OpencodeTurnUsage::new("openai/gpt-6.1-sol");
        usage.observe(decode_opencode_usage(&parse_event(line).unwrap()).unwrap());
        let turn = OpencodeTurn {
            worker: "verify",
            session_id: "ses_a",
            store: None,
            turn_id: None,
        };
        record_opencode_verify(&turn, &usage.take(), &file);
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        assert_eq!(text.lines().count(), 1);
        let row: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(row["kind"], "verify");
        assert_eq!(row["harness"], "opencode");
        assert_eq!(row["session_id"], "ses_a");
        assert_eq!(row["cost_usd"], 0.5);
        assert!(row["store"].is_null());
    }
}
