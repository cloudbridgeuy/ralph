use crate::chat::memory::{append_usage, usage_path};
use crate::warn::warn_if_err;
use ralph_core::chat_memory::{worker_usage_record, CallKind};
use ralph_core::worker::{parse_stop_session, subagents_dir, transcript_usage};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub struct UsageTarget {
    pub worker: String,
    pub seen: PathBuf,
}

fn read_or_empty(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        other => other,
    }
}

fn is_subagent_transcript(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("agent-") && name.ends_with(".jsonl"))
}

fn subagent_lines(dir: &Path) -> io::Result<String> {
    let entries = match fs::read_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        other => other?,
    };
    let mut text = String::new();
    for entry in entries {
        let path = entry?.path();
        if is_subagent_transcript(&path) {
            text.push_str(&read_or_empty(&path)?);
            text.push('\n');
        }
    }
    Ok(text)
}

fn record(
    target: &UsageTarget,
    payload: &str,
    turn_id: Option<u64>,
    usage_file: &Path,
) -> io::Result<()> {
    let session = parse_stop_session(payload).map_err(io::Error::other)?;
    let mut lines = read_or_empty(&session.transcript_path)?;
    lines.push('\n');
    lines.push_str(&subagent_lines(&subagents_dir(&session.transcript_path))?);
    let seen: HashSet<String> = read_or_empty(&target.seen)?
        .lines()
        .map(str::to_string)
        .collect();
    let found = transcript_usage(&lines, &seen);
    let at = chrono::Utc::now().to_rfc3339();
    for row in &found.rows {
        let kind = CallKind::Worker {
            worker_id: target.worker.clone(),
            turn_id,
        };
        let session_id = Some(session.session_id.clone());
        append_usage(
            usage_file,
            &worker_usage_record(kind, at.clone(), session_id, row),
        )?;
    }
    if found.recorded.is_empty() {
        return Ok(());
    }
    if let Some(dir) = target.seen.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&target.seen)?;
    file.write_all(format!("{}\n", found.recorded.join("\n")).as_bytes())
}

pub fn record_or_warn(target: &UsageTarget, payload: &str, turn_id: Option<u64>) {
    warn_if_err(
        record(target, payload, turn_id, &usage_path()),
        "Failed to record worker usage",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(id: &str, model: &str, stop: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"id":"{id}","model":"{model}","stop_reason":{stop},"usage":{{"input_tokens":1,"output_tokens":2}}}}}}"#
        )
    }

    #[test]
    fn main_and_subagent_transcripts_are_recorded_once() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("s1.jsonl");
        let subagents = dir.path().join("s1").join("subagents");
        fs::create_dir_all(&subagents).unwrap();
        fs::write(&transcript, assistant("m1", "opus", r#""end_turn""#)).unwrap();
        fs::write(
            subagents.join("agent-a.jsonl"),
            assistant("m2", "haiku", r#""end_turn""#),
        )
        .unwrap();
        fs::write(
            subagents.join("notes.txt"),
            assistant("m3", "haiku", r#""end_turn""#),
        )
        .unwrap();
        let target = UsageTarget {
            worker: "w1".into(),
            seen: dir.path().join("workers").join("w1.usage-seen"),
        };
        let usage_file = dir.path().join("usage.jsonl");
        let payload = format!(
            r#"{{"session_id":"s1","transcript_path":"{}"}}"#,
            transcript.display()
        );
        record(&target, &payload, Some(3), &usage_file).unwrap();
        record(&target, &payload, Some(4), &usage_file).unwrap();
        let written = fs::read_to_string(&usage_file).unwrap();
        assert_eq!(written.lines().count(), 2);
        assert!(written.contains(r#""kind":"worker""#));
        assert!(written.contains(r#""turn_id":3"#));
        assert_eq!(
            fs::read_to_string(&target.seen).unwrap(),
            "m1
m2
"
        );
    }
}
