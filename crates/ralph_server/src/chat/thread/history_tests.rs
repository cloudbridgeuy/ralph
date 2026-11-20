use super::*;
use crate::history_log::write_turn;
use ralph_core::chat_memory::{history_path, parse_tool_call};
use serde_json::{json, Value};
use std::fs;

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

fn thread_over_tagged_main(tagged: &[usize]) -> tempfile::TempDir {
    let main = tempfile::tempdir().unwrap();
    let entry = |n: usize| {
        format!(
            "{{\"role\":\"user\",\"at\":\"2026-10-20T10:00:0{n}+00:00\",\"text\":\"ship {n}\"}}"
        )
    };
    let log: String = (0..4).map(|n| format!("{}\n", entry(n))).collect();
    fs::write(main.path().join("log.jsonl"), log).unwrap();
    let tag = |n: usize| {
        format!(
            "{{\"v\":1,\"n\":{n},\"rule\":true,\"approval\":false,\"model\":\"m\",\"prompt\":\"v1\",\"at\":\"t\"}}\n"
        )
    };
    let tags: String = tagged.iter().map(|n| tag(*n)).collect();
    fs::write(main.path().join("tags.jsonl"), tags).unwrap();
    let thread = main.path().join("threads").join("auth");
    fs::create_dir_all(&thread).unwrap();
    fs::write(
        thread.join("thread.json"),
        "{\"base\":2,\"created\":\"2026-10-20T10:00:00+00:00\"}",
    )
    .unwrap();
    fs::write(thread.join("log.jsonl"), format!("{}\n", entry(5))).unwrap();
    main
}

#[test]
fn a_tagged_search_from_a_thread_lists_main_era_entries_tagged_in_mains_tags() {
    let main = thread_over_tagged_main(&[1, 3]);
    let text = call(
        &main.path().join("threads").join("auth"),
        "search",
        json!({"query": "ship", "tag": "rule", "thread": "main"}),
        CallSource::Mcp,
    )
    .unwrap();
    assert!(text.contains("#1 "), "{text}");
    assert!(!text.contains("#0 "), "{text}");
    assert!(!text.contains("#3 "), "{text}");
}

#[test]
fn a_tag_only_search_from_a_thread_lists_tagged_main_era_entries_newest_first() {
    let main = thread_over_tagged_main(&[0, 1, 3]);
    let text = call(
        &main.path().join("threads").join("auth"),
        "search",
        json!({"tag": "rule", "thread": "main"}),
        CallSource::Mcp,
    )
    .unwrap();
    let one = text.find("#1 ").unwrap();
    let zero = text.find("#0 ").unwrap();
    assert!(one < zero, "{text}");
    assert!(!text.contains("#3 "), "{text}");
    assert!(
        text.trim_end().ends_with(
            "(tags exist only for messages written after tagging began; older messages are not listed)"
        ),
        "{text}"
    );
}

fn thread_call(main: &tempfile::TempDir, tool: &str, args: Value) -> String {
    call(
        &main.path().join("threads").join("auth"),
        tool,
        args,
        CallSource::Cli,
    )
    .unwrap()
}

#[test]
fn a_cli_zoom_and_read_of_main_from_a_thread_show_mains_marks() {
    let main = thread_over_tagged_main(&[1]);
    let zoom = thread_call(&main, "zoom", json!({"id": "1", "thread": "main"}));
    assert!(zoom.contains("#1 user [rule] "), "{zoom}");
    let read = thread_call(&main, "read", json!({"id": "0-1", "thread": "main"}));
    assert!(read.contains("#1 · user [rule] "), "{read}");
    assert!(read.contains("#0 · user · "), "{read}");
}

#[test]
fn a_thread_scope_read_uses_the_thread_tags_not_mains() {
    let main = thread_over_tagged_main(&[2, 3]);
    let thread = main.path().join("threads").join("auth");
    let own = |tags: &str| {
        fs::write(thread.join("tags.jsonl"), tags).unwrap();
        thread_call(&main, "read", json!({"id": "auth:0"}))
    };
    assert!(!own("").contains("[rule]"));
    let row = "{\"v\":1,\"n\":0,\"rule\":true,\"approval\":false,\"model\":\"m\",\"prompt\":\"v1\",\"at\":\"t\"}\n";
    assert!(own(row).contains("[rule]"));
}
