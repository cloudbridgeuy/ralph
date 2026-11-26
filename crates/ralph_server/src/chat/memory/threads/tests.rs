use super::*;
use crate::chat::memory::Store;
use crate::chat::thread::thread_dir;
use ralph_core::chat_memory::Role;
use ralph_core::chat_thread::{Merged, Parked, ThreadName, THREAD_FILE};
use ralph_core::chat_thread_log::{tombstone_line, DROPPED_FILE};
use std::fs::{self, File};
use std::time::{Duration, UNIX_EPOCH};

fn name(s: &str) -> ThreadName {
    ThreadName::parse(s).unwrap()
}

fn meta(base: usize, created: &str) -> ThreadMeta {
    ThreadMeta {
        base,
        created: created.to_string(),
        merged: None,
        parked: None,
        auto: None,
        model: None,
        closed: false,
    }
}

fn seed(main: &Path, thread: &str, meta: &ThreadMeta, messages: usize, mtime_secs: u64) {
    let dir = thread_dir(main, &name(thread));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(THREAD_FILE), serde_json::to_string(meta).unwrap()).unwrap();
    let store = Store::new(dir.clone());
    for i in 0..messages {
        store.append(Role::User, &format!("note {i}")).unwrap();
    }
    File::options()
        .write(true)
        .open(dir.join("log.jsonl"))
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(mtime_secs))
        .unwrap();
}

fn seeded_store() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let main = root.path();
    let store = Store::new(main.to_path_buf());
    for i in 0..4 {
        store.append(Role::User, &format!("main {i}")).unwrap();
    }
    seed(
        main,
        "lint-sweep",
        &meta(2, "2026-01-02T00:00:00Z"),
        3,
        1_800_000_000,
    );
    let mut parked = meta(1, "2026-01-01T00:00:00Z");
    parked.parked = Some(Parked {
        at: "2026-01-03T00:00:00Z".to_string(),
    });
    seed(main, "docs-pass", &parked, 1, 1_800_000_100);
    let mut merged = meta(3, "2026-01-04T00:00:00Z");
    merged.merged = Some(Merged {
        at: "2026-01-05T00:00:00Z".to_string(),
        digest: "done".to_string(),
        landed: Some(4),
    });
    seed(main, "cache-fix", &merged, 2, 1_800_000_200);
    let gone = name("old-idea");
    let line = tombstone_line(
        &gone,
        &meta(2, "2025-12-30T00:00:00Z"),
        "2026-01-06T00:00:00Z",
    );
    fs::write(main.join("threads").join(DROPPED_FILE), line).unwrap();
    root
}

fn rendered(snapshot: &ThreadsSnapshot) -> String {
    let mut out = format!("head_messages {}\n", snapshot.head_messages);
    for row in &snapshot.rows {
        out.push_str(&format!(
            "row {} {:?} base={} created={} active={} messages={} workers={} unread={}\n",
            row.name,
            row.state,
            row.base,
            row.created,
            row.active.as_deref().unwrap_or("-"),
            row.messages,
            row.workers,
            row.unread
        ));
    }
    let mut live: Vec<String> = snapshot
        .live
        .iter()
        .map(|(name, meta)| format!("live {name} base={}\n", meta.base))
        .collect();
    live.sort();
    out.extend(live);
    for tombstone in &snapshot.tombstones {
        out.push_str(&format!(
            "tombstone {} base={} created={} dropped_at={}\n",
            tombstone.name, tombstone.base, tombstone.created, tombstone.dropped_at
        ));
    }
    out
}

fn assert_golden(file: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(file);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(expected, actual, "golden {} differs", path.display());
}

#[test]
fn thread_rows_golden() {
    let root = seeded_store();
    let snapshot = thread_rows(root.path()).unwrap();
    assert_golden("thread_rows.txt", &rendered(&snapshot));
}

#[test]
fn a_store_with_no_threads_gives_an_empty_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let snapshot = thread_rows(&root.path().join("missing")).unwrap();
    assert_eq!(snapshot.head_messages, 0);
    assert!(snapshot.rows.is_empty());
    assert!(snapshot.live.is_empty());
    assert!(snapshot.tombstones.is_empty());
}
