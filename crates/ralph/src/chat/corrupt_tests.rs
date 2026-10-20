use super::index_files::open_archive;
use super::memory::Store;
use super::monitor::fresh_warnings;
use ralph_core::chat_memory::{
    encode_entry, nap_step, pending, Entry, Mode, NapStep, Node, RangeRequest, Tree, CORRUPT_TEXT,
};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

fn entry(i: usize, len: usize) -> Entry {
    Entry {
        role: ralph_core::chat_memory::Role::User,
        at: format!("2026-01-01T00:00:{:02}Z", i % 60),
        text: format!("m{i} {}", "x".repeat(len)),
        corrupt: false,
    }
}

fn write_log(dir: &Path, parts: &[String]) {
    fs::write(dir.join("log.jsonl"), parts.concat()).unwrap();
}

fn line(i: usize, len: usize) -> String {
    encode_entry(&entry(i, len)).unwrap()
}

fn raw(from: usize, to: usize) -> RangeRequest {
    RangeRequest {
        from,
        to,
        mode: Mode::Raw,
        budget: 16384,
        cursor: None,
    }
}

fn naps_every_offered_range(entries: &[Entry]) -> (Vec<Node>, Vec<String>) {
    let mut tree = Tree::new();
    let mut offered = Vec::new();
    let mut prompts = Vec::new();
    loop {
        let todo = pending(entries, &tree);
        if todo.is_empty() {
            return (offered, prompts);
        }
        for node in todo {
            match nap_step(entries, &tree, node) {
                NapStep::Ask(prompt) => {
                    offered.push(node);
                    prompts.push(prompt);
                    tree.insert(node, "s".to_string());
                }
                other => panic!("{} not askable: {other:?}", node.id()),
            }
        }
    }
}

#[test]
fn no_nap_input_holds_a_placeholder_when_the_log_has_a_bad_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut parts: Vec<String> = (0..5).map(|i| line(i, 600)).collect();
    parts.push("not json\n".to_string());
    parts.extend((6..8).map(|i| line(i, 600)));
    write_log(dir.path(), &parts);
    let (entries, tree) = Store::new(dir.path().to_path_buf())
        .load_settled(|| ())
        .unwrap();
    assert_eq!(entries.len(), 8);
    assert!(entries[5].is_corrupt() && !entries[6].is_corrupt());
    let (offered, prompts) = naps_every_offered_range(&entries);
    assert!(tree.is_empty());
    assert!(offered.contains(&Node { lo: 0, hi: 4 }));
    assert!(offered.contains(&Node { lo: 6, hi: 8 }));
    assert!(offered.contains(&Node { lo: 6, hi: 7 }));
    assert!(offered.iter().all(|node| !(node.lo..node.hi).contains(&5)));
    for prompt in &prompts {
        assert!(!prompt.contains(CORRUPT_TEXT), "{prompt}");
        assert!(!prompt.lines().any(|l| l.starts_with("#5 ")), "{prompt}");
    }
    assert_eq!(prompts.len(), offered.len());
}

#[test]
fn a_bad_line_over_the_chunk_size_keeps_ids_and_later_entries() {
    let dir = tempfile::tempdir().unwrap();
    let long_bad = format!("{{\"role\":\"user\",\"text\":\"{}\n", "z".repeat(10_000));
    write_log(
        dir.path(),
        &[line(0, 5), line(1, 5), long_bad.clone(), line(3, 5)],
    );
    let store = Store::new(dir.path().to_path_buf());
    let (entries, _, warnings) = store.load_quiet().unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries[2].is_corrupt());
    assert_eq!(entries[3], entry(3, 5));
    let at = line(0, 5).len() + line(1, 5).len();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with(&format!("chat log entry 2 at byte {at} is corrupt")),
        "{}",
        warnings[0]
    );
    assert!(warnings[0].len() < 1_000, "{}", warnings[0].len());
    let page = store.read(&raw(0, 3)).unwrap().unwrap();
    assert!(
        page.contains("#2 · worker ·  · 15 chars\n[corrupt entry]"),
        "{page}"
    );
    assert!(page.contains("#3 · user · "), "{page}");
    assert!(page.contains(&entry(3, 5).text), "{page}");
    let mut archive = open_archive(dir.path()).unwrap();
    assert_eq!(archive.count(), 4);
    assert_eq!(archive.entries(0, 4).unwrap(), entries);
    assert_eq!(archive.warnings(), warnings);
}

#[test]
fn a_log_that_is_one_bad_line_loads_as_one_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    write_log(dir.path(), &["not json\n".to_string()]);
    let store = Store::new(dir.path().to_path_buf());
    let (entries, tree, warnings) = store.load_quiet().unwrap();
    assert_eq!(entries, [Entry::corrupt()]);
    assert!(tree.is_empty());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with("chat log entry 0 at byte 0 is corrupt"),
        "{}",
        warnings[0]
    );
    let page = store.read(&raw(0, 1)).unwrap().unwrap();
    assert!(
        page.contains("#0 · worker ·  · 15 chars\n[corrupt entry]"),
        "{page}"
    );
    let mut archive = open_archive(dir.path()).unwrap();
    assert_eq!(archive.entries(0, 1).unwrap(), entries);
    assert_eq!(archive.warnings(), warnings);
    store
        .append(ralph_core::chat_memory::Role::User, "hello")
        .unwrap();
    let (after, _, again) = store.load_quiet().unwrap();
    assert_eq!(after.len(), 2);
    assert!(after[0].is_corrupt() && after[1].text == "hello");
    assert_eq!(again.len(), 1);
}

#[test]
fn the_status_bar_warns_once_per_bad_line_across_repeated_loads() {
    let dir = tempfile::tempdir().unwrap();
    write_log(
        dir.path(),
        &[
            line(0, 5),
            "bad one\n".to_string(),
            line(2, 5),
            "bad two\n".to_string(),
        ],
    );
    let store = Store::new(dir.path().to_path_buf());
    let mut seen = HashSet::new();
    let mut printed = Vec::new();
    for _ in 0..4 {
        let (_, _, warnings) = store.load_quiet().unwrap();
        assert_eq!(warnings.len(), 2);
        printed.push(fresh_warnings(&mut seen, warnings).len());
    }
    assert_eq!(printed, [2, 0, 0, 0]);
    let mut log = fs::read(dir.path().join("log.jsonl")).unwrap();
    log.extend_from_slice(b"bad three\n");
    fs::write(dir.path().join("log.jsonl"), log).unwrap();
    let (_, _, warnings) = store.load_quiet().unwrap();
    assert_eq!(warnings.len(), 3);
    let fresh = fresh_warnings(&mut seen, warnings);
    assert_eq!(fresh.len(), 1, "{fresh:?}");
    assert!(
        fresh[0].starts_with("chat log entry 4 at byte "),
        "{}",
        fresh[0]
    );
}

#[test]
fn fresh_warnings_returns_each_message_only_the_first_time() {
    let mut seen = HashSet::new();
    let batch = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        fresh_warnings(&mut seen, batch(&["a", "b", "a"])),
        ["a", "b"]
    );
    assert_eq!(fresh_warnings(&mut seen, batch(&["b", "c"])), ["c"]);
    assert!(fresh_warnings(&mut seen, batch(&[])).is_empty());
}
