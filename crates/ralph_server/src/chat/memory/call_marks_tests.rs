use super::*;
use ralph_core::chat_memory::parse_tool_call;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

fn store_with_log() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("s"));
    fs::create_dir_all(store.dir()).unwrap();
    let log: String = (0..4)
        .map(|n| {
            format!(
                "{{\"role\":\"user\",\"at\":\"2026-10-20T10:00:0{n}+00:00\",\"text\":\"ship {n}\"}}\n"
            )
        })
        .collect();
    fs::write(store.dir().join("log.jsonl"), log).unwrap();
    (dir, store)
}

fn tag_row(n: usize, extra: &str) -> String {
    format!(
        "{{\"v\":1,\"n\":{n},\"rule\":true,\"approval\":false,\"model\":\"m\",\"prompt\":\"v1\",\"at\":\"t\"{extra}}}\n"
    )
}

fn write_tags(dir: &Path, text: &str) {
    fs::write(dir.join("tags.jsonl"), text).unwrap();
}

fn run(store: &Store, name: &str, args: Value) -> Result<String, String> {
    store
        .call(
            &parse_tool_call(name, &args).unwrap().tool,
            Reach::default(),
        )
        .map(|answer| answer.text)
}

fn line_of(text: &str, id: usize) -> &str {
    let head = format!("#{id} ");
    text.lines()
        .find(|line| line.starts_with(&head))
        .unwrap_or_default()
}

fn all_tools() -> Vec<(&'static str, Value)> {
    vec![
        ("zoom", json!({"id": "1"})),
        ("read", json!({"id": "0-3"})),
        ("recall", json!({"pattern": "ship"})),
        ("search", json!({"query": "ship"})),
    ]
}

#[test]
fn every_tool_marks_a_tagged_entry() {
    let (_dir, store) = store_with_log();
    write_tags(store.dir(), &tag_row(1, ""));
    for (name, args) in all_tools() {
        let text = run(&store, name, args).unwrap();
        assert!(
            line_of(&text, 1).contains(" user [rule] "),
            "{name}: {text}"
        );
        assert!(!line_of(&text, 0).contains('['), "{name}: {text}");
    }
}

#[test]
fn a_missing_tags_file_leaves_the_output_unmarked() {
    let (_dir, store) = store_with_log();
    for (name, args) in all_tools() {
        let text = run(&store, name, args).unwrap();
        assert!(!text.contains("[rule]"), "{name}: {text}");
    }
}

#[test]
fn a_garbled_tags_file_is_no_error_and_marks_valid_rows_only() {
    let (_dir, store) = store_with_log();
    write_tags(
        store.dir(),
        &format!("garbage\n{}{{\"v\":1,\"n\":2,\"ru", tag_row(1, "")),
    );
    for (name, args) in all_tools() {
        let text = run(&store, name, args).unwrap();
        assert!(
            line_of(&text, 1).contains(" user [rule] "),
            "{name}: {text}"
        );
        assert!(!line_of(&text, 2).contains('['), "{name}: {text}");
    }
}

#[test]
fn failed_and_skipped_rows_carry_no_mark() {
    let (_dir, store) = store_with_log();
    write_tags(
        store.dir(),
        &format!(
            "{}{}",
            tag_row(1, ",\"failed\":true"),
            tag_row(2, ",\"skipped\":true")
        ),
    );
    for (name, args) in all_tools() {
        let text = run(&store, name, args).unwrap();
        assert!(!text.contains("[rule]"), "{name}: {text}");
    }
}

#[test]
fn an_unreadable_tags_file_fails_only_the_tag_search() {
    let (_dir, store) = store_with_log();
    fs::create_dir(store.dir().join("tags.jsonl")).unwrap();
    for (name, args) in all_tools() {
        let text = run(&store, name, args).unwrap();
        assert!(!text.contains("[rule]"), "{name}: {text}");
    }
    let error = run(&store, "search", json!({"query": "ship", "tag": "rule"})).unwrap_err();
    assert!(error.starts_with("Could not read chat memory: "), "{error}");
}

#[test]
fn a_tag_search_marks_its_hits_from_the_same_tags() {
    let (_dir, store) = store_with_log();
    write_tags(
        store.dir(),
        &format!("{}{}", tag_row(1, ""), tag_row(3, "")),
    );
    let text = run(&store, "search", json!({"query": "ship", "tag": "rule"})).unwrap();
    assert!(line_of(&text, 1).contains(" user [rule] "), "{text}");
    assert!(line_of(&text, 3).contains(" user [rule] "), "{text}");
    assert!(!text.contains("#0 "), "{text}");
}
