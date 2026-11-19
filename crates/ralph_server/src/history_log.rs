use crate::outcome_log::open_log;
use crate::private_file::{check_path, write_private};
use crate::warn::warn_if_err;
use crate::workers::usage_store;
use ralph_core::chat_memory::{
    count_turn_calls, encode_history, history_call_facts, history_call_row, history_error_row,
    history_path, history_turn_row, next_turn_number, parse_turn_id, turn_id, CallContext,
    CallSource, ErrorScope, HistoryKey, HistoryRow, MalformedCall, MemoryCall, TurnFacts,
};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub struct TurnMark {
    pub id: String,
    offset: u64,
}

pub fn turn_path(store: &Path) -> PathBuf {
    store.join("TURN")
}

pub fn read_turn(store: &Path) -> Option<String> {
    let text = fs::read_to_string(turn_path(store)).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub fn write_turn(store: &Path, id: &str) -> io::Result<()> {
    fs::create_dir_all(store)?;
    write_private(turn_path(store), id.as_bytes()).map(drop)
}

pub fn clear_turn(store: &Path) {
    let path = turn_path(store);
    if matches!(check_path(&path), Ok(())) && path.exists() {
        warn_if_err(fs::remove_file(&path), "clearing the chat turn marker");
    }
}

fn history_len(store: &Path) -> u64 {
    fs::metadata(history_path(store)).map_or(0, |meta| meta.len())
}

fn history_after(store: &Path, offset: u64) -> String {
    let read = || -> io::Result<Vec<u8>> {
        let mut file = File::open(history_path(store))?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    String::from_utf8_lossy(&read().unwrap_or_default()).into_owned()
}

pub fn begin_turn(store: &Path, run: &str) -> TurnMark {
    let number = next_turn_number(read_turn(store).as_deref(), run);
    let id = turn_id(run, number);
    warn_if_err(write_turn(store, &id), "recording the chat turn marker");
    TurnMark {
        id,
        offset: history_len(store),
    }
}

pub fn end_turn(store: &Path, run: &str, mark: &TurnMark, scope: &str, view_ids: Vec<String>) {
    let facts = TurnFacts {
        scope: scope.to_string(),
        history_calls: count_turn_calls(&history_after(store, mark.offset), &mark.id),
        view_ids,
    };
    append_row(store, Some(run), Some(&mark.id), |key| {
        history_turn_row(key, &facts)
    });
}

fn current_turn(store: &Path, source: CallSource) -> Option<String> {
    match source {
        CallSource::Cli => None,
        CallSource::Mcp | CallSource::Slash => read_turn(store),
    }
}

fn append_row(
    store: &Path,
    run: Option<&str>,
    turn: Option<&str>,
    build: impl FnOnce(&HistoryKey) -> HistoryRow,
) {
    let canonical = usage_store(store);
    let row = build(&HistoryKey {
        store: &canonical,
        run,
        at: chrono::Utc::now(),
        turn,
    });
    warn_if_err(append(store, &row), "recording history call");
}

fn append_call_row(
    store: &Path,
    source: CallSource,
    build: impl FnOnce(&HistoryKey) -> HistoryRow,
) {
    let current = current_turn(store, source);
    let run = current
        .as_deref()
        .and_then(parse_turn_id)
        .map(|(run, _)| run);
    append_row(store, run, current.as_deref(), build);
}

fn append(store: &Path, row: &HistoryRow) -> io::Result<()> {
    if !store.is_dir() {
        return Ok(());
    }
    let line = encode_history(row).map_err(io::Error::other)?;
    let path = history_path(store);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    check_path(&path)?;
    open_log(&path)?.write_all(line.as_bytes())
}

pub fn record_call(
    store: &Path,
    call: &MemoryCall,
    result: &Result<String, String>,
    ctx: CallContext,
) {
    let facts = history_call_facts(call, result, ctx);
    append_call_row(store, ctx.source, |key| history_call_row(key, &facts));
}

pub fn record_error(store: &Path, bad: &MalformedCall, scope: ErrorScope, ctx: CallContext) {
    append_call_row(store, ctx.source, |key| {
        history_error_row(key, bad, scope, ctx)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_memory::{
        history_path, parse_tool_call, CallMetrics, CallSource, HistoryTool, MemoryCall,
    };
    use serde_json::{json, Value};
    use std::fs;
    use std::time::Duration;

    fn ctx(source: CallSource) -> CallContext {
        CallContext {
            source,
            took: Duration::from_millis(3),
            metrics: CallMetrics::default(),
        }
    }

    fn bad(tool: HistoryTool, raw: Value, message: &str) -> MalformedCall {
        MalformedCall {
            tool,
            raw,
            message: message.to_string(),
        }
    }

    fn zoom() -> MemoryCall {
        parse_tool_call("zoom", &json!({"id": "4-7"})).unwrap()
    }

    fn rows(store: &Path) -> Vec<Value> {
        fs::read_to_string(history_path(store))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn two_calls_make_two_json_lines() {
        let dir = tempfile::tempdir().unwrap();
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Cli));
        record_call(dir.path(), &zoom(), &Ok("b".into()), ctx(CallSource::Cli));
        let text = fs::read_to_string(history_path(dir.path())).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.ends_with('\n'));
        let first = &rows(dir.path())[0];
        assert_eq!(first["event"], "call");
        assert_eq!(first["tool"], "zoom");
        assert_eq!(first["source"], "cli");
        assert_eq!(first["outcome"], "ok");
        assert_eq!(first["result_bytes"], 1);
        assert_eq!(first["store"], crate::workers::usage_store(dir.path()));
    }

    #[test]
    fn an_mcp_call_carries_the_run_and_turn_from_the_turn_file() {
        let dir = tempfile::tempdir().unwrap();
        write_turn(dir.path(), "run-9#4").unwrap();
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Mcp));
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Slash));
        for row in rows(dir.path()) {
            assert_eq!(row["run"], "run-9");
            assert_eq!(row["turn"], "run-9#4");
        }
    }

    #[test]
    fn a_cli_call_has_no_turn_even_when_a_turn_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        write_turn(dir.path(), "run-9#4").unwrap();
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Cli));
        let row = &rows(dir.path())[0];
        assert!(row.get("run").is_none());
        assert!(row.get("turn").is_none());
    }

    #[test]
    fn a_missing_or_empty_turn_file_leaves_run_and_turn_out() {
        let dir = tempfile::tempdir().unwrap();
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Mcp));
        fs::write(turn_path(dir.path()), "").unwrap();
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Mcp));
        for row in rows(dir.path()) {
            assert!(row.get("run").is_none());
            assert!(row.get("turn").is_none());
        }
    }

    #[test]
    fn an_error_row_records_the_message_and_the_tool() {
        let dir = tempfile::tempdir().unwrap();
        record_error(
            dir.path(),
            &bad(HistoryTool::Zoom, json!({"id": "9"}), "bad id"),
            ErrorScope::AsTyped,
            ctx(CallSource::Slash),
        );
        let row = &rows(dir.path())[0];
        assert_eq!(row["outcome"], "error");
        assert_eq!(row["error"], "bad id");
        assert_eq!(row["source"], "slash");
        assert_eq!(row["args"], json!({"id": "9"}));
    }

    #[test]
    fn a_failed_append_is_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("analytics"), "").unwrap();
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Mcp));
        record_error(
            dir.path(),
            &bad(HistoryTool::Read, Value::Null, "x"),
            ErrorScope::AsTyped,
            ctx(CallSource::Mcp),
        );
        assert!(dir.path().join("analytics").is_file());
    }

    #[test]
    fn a_symlinked_log_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = history_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Cli));
        assert_eq!(fs::read_to_string(&target).unwrap(), "");
    }

    #[test]
    fn begin_turn_numbers_the_turns_of_a_run_on() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(begin_turn(dir.path(), "r1").id, "r1#1");
        assert_eq!(read_turn(dir.path()).as_deref(), Some("r1#1"));
        assert_eq!(begin_turn(dir.path(), "r1").id, "r1#2");
        assert_eq!(begin_turn(dir.path(), "r2").id, "r2#1");
        assert_eq!(read_turn(dir.path()).as_deref(), Some("r2#1"));
    }

    #[test]
    fn clear_turn_removes_the_file_and_is_quiet_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        clear_turn(dir.path());
        write_turn(dir.path(), "r1#1").unwrap();
        clear_turn(dir.path());
        assert!(!turn_path(dir.path()).exists());
        assert_eq!(read_turn(dir.path()), None);
    }

    #[test]
    fn a_turn_row_counts_this_turns_calls_and_lists_the_view() {
        let dir = tempfile::tempdir().unwrap();
        let earlier = begin_turn(dir.path(), "r1");
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Mcp));
        end_turn(dir.path(), "r1", &earlier, "main", Vec::new());
        let mark = begin_turn(dir.path(), "r1");
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Mcp));
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Slash));
        record_call(dir.path(), &zoom(), &Ok("a".into()), ctx(CallSource::Cli));
        end_turn(
            dir.path(),
            "r1",
            &mark,
            "auth",
            vec!["0-15".into(), "16".into()],
        );
        let all = rows(dir.path());
        let turn = all.last().unwrap();
        assert_eq!(turn["event"], "turn");
        assert_eq!(turn["run"], "r1");
        assert_eq!(turn["turn"], "r1#2");
        assert_eq!(turn["scope"], "auth");
        assert_eq!(turn["history_calls"], 1);
        assert_eq!(turn["view_ids"], json!(["0-15", "16"]));
        assert_eq!(all[1]["history_calls"], 1);
    }

    #[test]
    fn a_turn_without_calls_still_gets_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let mark = begin_turn(dir.path(), "r1");
        end_turn(dir.path(), "r1", &mark, "main", Vec::new());
        let all = rows(dir.path());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["history_calls"], 0);
        assert_eq!(all[0]["view_ids"], json!([]));
    }
}
