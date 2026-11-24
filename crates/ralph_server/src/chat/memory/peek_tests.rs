use super::peek::{list_threads, peek, thread_exists, ThreadInfo};
use ralph_core::chat_memory::{encode_entry, encode_record, NewEntry, Node, Role, CORRUPT_TEXT};
use ralph_core::chat_thread::ThreadName;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

fn entry_line(text: &str) -> String {
    encode_entry(&NewEntry {
        role: Role::User,
        at: "2026-01-01T00:00:00Z".to_string(),
        text: text.to_string(),
    })
    .unwrap()
}

fn node_line(lo: usize, hi: usize) -> String {
    encode_record(Node::new(lo, hi).unwrap(), "sum").unwrap()
}

fn write_log(dir: &Path, texts: &[&str]) {
    let log: String = texts.iter().map(|t| entry_line(t)).collect();
    fs::write(dir.join("log.jsonl"), log).unwrap();
}

type Snapshot = BTreeMap<String, Option<(Vec<u8>, std::time::SystemTime)>>;

fn snapshot(root: &Path) -> Snapshot {
    fn walk(root: &Path, dir: &Path, out: &mut Snapshot) {
        for item in fs::read_dir(dir).unwrap() {
            let path = item.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            if path.is_dir() {
                out.insert(rel, None);
                walk(root, &path, out);
            } else {
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                out.insert(rel, Some((fs::read(&path).unwrap(), modified)));
            }
        }
    }
    let mut out = Snapshot::new();
    walk(root, root, &mut out);
    out
}

fn full_store(dir: &Path) {
    write_log(dir, &["a", "b", "c", "d"]);
    fs::write(dir.join("tree.jsonl"), node_line(0, 2)).unwrap();
    fs::write(dir.join("log.idx"), b"idx").unwrap();
    fs::write(dir.join("tree.idx"), b"idx").unwrap();
    fs::write(dir.join("tags.jsonl"), b"{}\n").unwrap();
    fs::create_dir_all(dir.join("analytics")).unwrap();
    fs::write(dir.join("analytics/history.jsonl"), b"h\n").unwrap();
    let thread = dir.join("threads/x");
    fs::create_dir_all(&thread).unwrap();
    fs::write(thread.join("thread.json"), r#"{"base":1,"created":"t"}"#).unwrap();
    write_log(&thread, &["t"]);
}

#[test]
fn half_written_last_log_line_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = entry_line("a");
    log.push_str(&entry_line("b"));
    log.push_str("{\"role\":\"user\",\"at\":\"x\",\"te");
    fs::write(dir.path().join("log.jsonl"), log).unwrap();
    assert_eq!(peek(dir.path()).unwrap().entries.len(), 2);
}

#[test]
fn half_written_last_tree_line_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    write_log(dir.path(), &["a", "b"]);
    let mut tree = node_line(0, 2);
    tree.push_str("{\"lo\":0,\"hi\":");
    fs::write(dir.path().join("tree.jsonl"), tree).unwrap();
    assert_eq!(peek(dir.path()).unwrap().tree.len(), 1);
}

#[test]
fn tree_ahead_of_log_is_hidden() {
    let dir = tempfile::tempdir().unwrap();
    write_log(dir.path(), &["a", "b"]);
    let tree = format!("{}{}", node_line(0, 2), node_line(2, 4));
    fs::write(dir.path().join("tree.jsonl"), tree).unwrap();
    let seen = peek(dir.path()).unwrap();
    assert_eq!(seen.tree.len(), 1);
    assert!(seen.tree.contains_key(&Node::new(0, 2).unwrap()));
}

#[test]
fn peek_and_list_threads_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    full_store(dir.path());
    let before = snapshot(dir.path());
    let seen = peek(dir.path()).unwrap();
    let listed = list_threads(dir.path()).unwrap();
    assert_eq!(seen.entries.len(), 4);
    assert_eq!(listed.len(), 1);
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn peek_creates_no_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    full_store(dir.path());
    peek(dir.path()).unwrap();
    list_threads(dir.path()).unwrap();
    assert!(!dir.path().join("LOCK").exists());
}

#[test]
fn peek_on_missing_dir_is_not_found_and_creates_nothing() {
    let parent = tempfile::tempdir().unwrap();
    let missing = parent.path().join("nope");
    let err = peek(&missing).err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(!missing.exists());
    assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn held_exclusive_lock_does_not_block_peek() {
    let dir = tempfile::tempdir().unwrap();
    write_log(dir.path(), &["a"]);
    let lock = fs::File::create(dir.path().join("LOCK")).unwrap();
    lock.lock().unwrap();
    let path = dir.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        tx.send(peek(&path).map(|p| p.entries.len())).ok();
    });
    let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(got.unwrap(), 1);
}

#[test]
fn corrupt_middle_line_keeps_index_and_shows_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let log = format!("{}not json\n{}", entry_line("a"), entry_line("c"));
    fs::write(dir.path().join("log.jsonl"), log).unwrap();
    let entries = peek(dir.path()).unwrap().entries;
    assert_eq!(entries.len(), 3);
    assert!(entries[1].corrupt);
    assert_eq!(entries[1].text, CORRUPT_TEXT);
    assert_eq!(entries[2].text, "c");
}

#[cfg(unix)]
#[test]
fn symlinked_log_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.jsonl");
    fs::write(&real, entry_line("a")).unwrap();
    std::os::unix::fs::symlink(&real, dir.path().join("log.jsonl")).unwrap();
    assert!(peek(dir.path()).is_err());
}

#[test]
fn list_threads_is_sorted_and_skips_bad_entries() {
    let dir = tempfile::tempdir().unwrap();
    let threads = dir.path().join("threads");
    let put = |name: &str, json: &str| {
        fs::create_dir_all(threads.join(name)).unwrap();
        fs::write(threads.join(name).join("thread.json"), json).unwrap();
    };
    put("zed", r#"{"base":0,"created":"t","parked":{"at":"t"}}"#);
    put(
        "alpha",
        r#"{"base":0,"created":"t","merged":{"at":"t","digest":"d"}}"#,
    );
    put("Bad_Name", r#"{"base":0,"created":"t"}"#);
    put("broken", "{nope");
    fs::create_dir_all(threads.join("empty")).unwrap();
    let listed = list_threads(dir.path()).unwrap();
    let want = vec![
        ThreadInfo {
            name: "alpha".to_string(),
            parked: false,
            merged: true,
        },
        ThreadInfo {
            name: "zed".to_string(),
            parked: true,
            merged: false,
        },
    ];
    assert_eq!(listed, want);
}

#[test]
fn list_threads_without_threads_dir_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert!(list_threads(dir.path()).unwrap().is_empty());
}

#[test]
fn thread_exists_checks_thread_json() {
    let dir = tempfile::tempdir().unwrap();
    full_store(dir.path());
    assert!(thread_exists(dir.path(), &ThreadName::parse("x").unwrap()));
    assert!(!thread_exists(dir.path(), &ThreadName::parse("y").unwrap()));
}

#[test]
fn thread_exists_requires_a_parseable_thread_json() {
    let dir = tempfile::tempdir().unwrap();
    let threads = dir.path().join("threads");
    let valid = r#"{"base":0,"created":"t"}"#;
    let cases = [
        ("corrupt", "{nope", false),
        ("empty", "", false),
        ("truncated", &valid[..valid.len() - 4], false),
        ("valid", valid, true),
    ];
    for (name, json, _) in cases {
        fs::create_dir_all(threads.join(name)).unwrap();
        fs::write(threads.join(name).join("thread.json"), json).unwrap();
    }
    let listed: Vec<String> = list_threads(dir.path())
        .unwrap()
        .into_iter()
        .map(|info| info.name)
        .collect();
    for (name, _, ok) in cases {
        let found = thread_exists(dir.path(), &ThreadName::parse(name).unwrap());
        assert_eq!(found, ok, "thread_exists {name}");
        assert_eq!(listed.iter().any(|n| n == name), ok, "listed {name}");
    }
}
