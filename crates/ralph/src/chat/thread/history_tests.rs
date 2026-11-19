use super::*;
use ralph_core::chat_memory::{history_path, parse_tool_call};
use ralph_server::history_log::write_turn;
use serde_json::{json, Value};

fn seeded() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let log = [
        "{\"role\":\"user\",\"at\":\"2026-10-20T10:00:00+00:00\",\"text\":\"ship GUZ-7090 first\"}",
        "{\"role\":\"assistant\",\"at\":\"2026-10-20T11:00:00+00:00\",\"text\":\"then ship GUZ-709\"}",
    ]
    .join("\n");
    fs::write(dir.path().join("log.jsonl"), format!("{log}\n")).unwrap();
    dir
}

fn call(dir: &Path, tool: &str, args: Value, source: CallSource) -> Result<String, String> {
    Memory::open(dir)
        .unwrap()
        .call(&parse_tool_call(tool, &args).unwrap(), source)
        .map(|answer| answer.text)
}

fn rows(dir: &Path) -> Vec<Value> {
    fs::read_to_string(history_path(dir))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_recall_logs_its_hits_scope_and_size() {
    let dir = seeded();
    let text = call(
        dir.path(),
        "recall",
        json!({"pattern": "ship"}),
        CallSource::Mcp,
    )
    .unwrap();
    let row = &rows(dir.path())[0];
    assert_eq!(row["source"], "mcp");
    assert_eq!(row["tool"], "recall");
    assert_eq!(row["scope"], "main");
    assert_eq!(row["outcome"], "ok");
    assert_eq!(row["hits"], 2);
    assert_eq!(row["result_bytes"], text.len());
    assert_eq!(row["args"], json!({"pattern": "ship"}));
}

#[test]
fn a_search_logs_its_query() {
    let dir = seeded();
    call(
        dir.path(),
        "search",
        json!({"query": "GUZ-709"}),
        CallSource::Slash,
    )
    .unwrap();
    let row = &rows(dir.path())[0];
    assert_eq!(row["tool"], "search");
    assert_eq!(row["args"]["query"], "GUZ-709");
}

#[test]
fn an_empty_search_logs_outcome_empty() {
    let dir = seeded();
    let text = call(
        dir.path(),
        "search",
        json!({"query": "zzz"}),
        CallSource::Cli,
    )
    .unwrap();
    assert!(text.starts_with("no hits for "));
    assert_eq!(rows(dir.path())[0]["outcome"], "empty");
}

#[test]
fn an_error_logs_its_text_and_the_result_is_unchanged() {
    let dir = seeded();
    let err = call(dir.path(), "zoom", json!({"id": "99"}), CallSource::Mcp).unwrap_err();
    let row = &rows(dir.path())[0];
    assert_eq!(row["outcome"], "error");
    assert_eq!(row["error"], err);
}

#[test]
fn each_source_is_tagged() {
    let dir = seeded();
    for (source, name) in [
        (CallSource::Mcp, "mcp"),
        (CallSource::Slash, "slash"),
        (CallSource::Cli, "cli"),
    ] {
        call(dir.path(), "zoom", json!({"id": "0"}), source).unwrap();
        assert_eq!(rows(dir.path()).last().unwrap()["source"], name);
    }
}

#[test]
fn a_cli_call_logs_no_turn_but_an_mcp_call_does() {
    let dir = seeded();
    write_turn(dir.path(), "run-1#5").unwrap();
    call(dir.path(), "zoom", json!({"id": "0"}), CallSource::Cli).unwrap();
    call(dir.path(), "zoom", json!({"id": "0"}), CallSource::Mcp).unwrap();
    let all = rows(dir.path());
    assert!(all[0].get("turn").is_none());
    assert_eq!(all[1]["run"], "run-1");
    assert_eq!(all[1]["turn"], "run-1#5");
}

#[test]
fn a_failed_append_does_not_change_the_result() {
    let plain = seeded();
    let blocked = seeded();
    fs::write(blocked.path().join("analytics"), "").unwrap();
    let zoom = |dir: &Path| call(dir, "zoom", json!({"id": "1"}), CallSource::Cli);
    assert_eq!(zoom(blocked.path()), zoom(plain.path()));
    assert!(rows(blocked.path()).is_empty());
}

#[test]
fn an_unforked_own_scope_logs_as_main() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("threads").join("auth");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("log.jsonl"),
        "{\"role\":\"user\",\"at\":\"t\",\"text\":\"hi\"}\n",
    )
    .unwrap();
    let memory = Memory {
        own: Store::new(dir.clone()),
        fork: None,
    };
    memory
        .call(
            &MemoryCall {
                scope: Scope::Own,
                tool: parse_tool_call("zoom", &json!({"id": "0"})).unwrap().tool,
            },
            CallSource::Cli,
        )
        .unwrap();
    assert_eq!(rows(&dir)[0]["scope"], "main");
}
