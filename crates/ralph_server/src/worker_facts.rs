use crate::chat::memory::peek;
use crate::opencode_worker::entry_ids;
use ralph_core::worker::{
    activity_path, blocked_path, decode_spawn_record, gone_path, spawn_record_path,
    unreported_path, waiting_path, ActivityMark, ActivityState, AGY_OWNER_DIR, CLAUDE_OWNER_DIR,
    CODEX_OWNER_DIR, CURSOR_OWNER_DIR, OPENCODE_OWNER_DIR,
};
use ralph_core::workers_view::{group_reports, ReportLine, WorkerFacts};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const OWNER_DIRS: [(&str, &str); 5] = [
    ("agy", AGY_OWNER_DIR),
    ("cursor", CURSOR_OWNER_DIR),
    ("claude", CLAUDE_OWNER_DIR),
    ("codex", CODEX_OWNER_DIR),
    ("opencode", OPENCODE_OWNER_DIR),
];

/// True only for a regular file below `store`. No part of the path under the
/// store may be a symlink, so a link in the store cannot pull in a file or a
/// directory from outside it. On unix the file must have one link, so a hard
/// link to a file elsewhere is refused too. The store path is used as given.
fn is_file(store: &Path, path: &Path) -> bool {
    let Ok(below) = path.strip_prefix(store) else {
        return false;
    };
    let mut current = store.to_path_buf();
    below.components().all(|part| {
        current.push(part);
        fs::symlink_metadata(&current).is_ok_and(|meta| !meta.file_type().is_symlink())
    }) && fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && single_link(&meta))
}

#[cfg(unix)]
fn single_link(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.nlink() == 1
}

#[cfg(not(unix))]
fn single_link(_: &fs::Metadata) -> bool {
    true
}

fn read_file(store: &Path, path: &Path) -> Option<String> {
    is_file(store, path).then(|| fs::read_to_string(path).ok())?
}

fn ticket_of(store: &Path, id: &str) -> Option<String> {
    decode_spawn_record(&read_file(store, &spawn_record_path(store, id))?)?.ticket
}

fn activity_of(store: &Path, id: &str) -> Option<ActivityState> {
    ActivityMark::parse(&read_file(store, &activity_path(store, id))?).map(|mark| mark.state)
}

fn last_reports(store: &Path) -> HashMap<String, ReportLine> {
    let Ok(data) = peek(store) else {
        return HashMap::new();
    };
    group_reports(&data.entries, &[])
        .into_iter()
        .filter_map(|(id, reports)| reports.into_iter().next().map(|report| (id, report)))
        .collect()
}

pub fn worker_facts(store: &Path) -> Vec<WorkerFacts> {
    let mut reports = last_reports(store);
    OWNER_DIRS
        .iter()
        .flat_map(|(harness, dir)| {
            entry_ids(store, dir)
                .into_iter()
                .filter(move |id| is_file(store, &store.join(dir).join(id)))
                .map(move |id| (*harness, id))
        })
        .map(|(harness, id)| WorkerFacts {
            harness: harness.to_string(),
            blocked: read_file(store, &blocked_path(store, &id)),
            waiting: is_file(store, &waiting_path(store, &id)),
            unreported: is_file(store, &unreported_path(store, &id)),
            gone: is_file(store, &gone_path(store, &id)),
            activity: activity_of(store, &id),
            ticket: ticket_of(store, &id),
            last_report: reports.remove(&id),
            id,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_memory::{encode_entry, NewEntry, Role};
    use ralph_core::worker::{
        activity_path, blocked_path, encode_spawn_record, gone_path, spawn_record_path,
        unreported_path, waiting_path, ActivityState, SpawnRecord, AGY_OWNER_DIR, CLAUDE_OWNER_DIR,
        CODEX_OWNER_DIR, CURSOR_OWNER_DIR, OPENCODE_OWNER_DIR,
    };
    use std::fs;
    use std::path::Path;

    fn touch(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn owner(store: &Path, dir: &str, id: &str) {
        touch(&store.join(dir).join(id), "");
    }

    fn find<'a>(facts: &'a [WorkerFacts], id: &str) -> &'a WorkerFacts {
        facts.iter().find(|facts| facts.id == id).unwrap()
    }

    #[test]
    fn an_empty_store_has_no_workers() {
        let store = tempfile::tempdir().unwrap();
        assert!(worker_facts(store.path()).is_empty());
    }

    #[test]
    fn workers_come_from_every_owner_directory_with_their_harness() {
        let store = tempfile::tempdir().unwrap();
        owner(store.path(), AGY_OWNER_DIR, "a1");
        owner(store.path(), CURSOR_OWNER_DIR, "c1");
        owner(store.path(), CLAUDE_OWNER_DIR, "k1");
        owner(store.path(), CODEX_OWNER_DIR, "x1");
        owner(store.path(), OPENCODE_OWNER_DIR, "o1");
        owner(store.path(), OPENCODE_OWNER_DIR, ".hidden");
        let facts = worker_facts(store.path());
        let found: Vec<(&str, &str)> = facts
            .iter()
            .map(|facts| (facts.id.as_str(), facts.harness.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("a1", "agy"),
                ("c1", "cursor"),
                ("k1", "claude"),
                ("x1", "codex"),
                ("o1", "opencode"),
            ]
        );
    }

    #[test]
    fn markers_and_activity_are_read_with_the_core_paths() {
        let store = tempfile::tempdir().unwrap();
        let root = store.path();
        for id in ["b", "w", "u", "g", "f", "plain"] {
            owner(root, CLAUDE_OWNER_DIR, id);
        }
        touch(&blocked_path(root, "b"), r#"{"request_id":"r1"}"#);
        touch(&waiting_path(root, "w"), "");
        touch(&unreported_path(root, "u"), "");
        touch(&gone_path(root, "g"), "left");
        touch(&activity_path(root, "f"), "7 failed");
        let facts = worker_facts(root);
        assert_eq!(
            find(&facts, "b").blocked.as_deref(),
            Some(r#"{"request_id":"r1"}"#)
        );
        assert!(find(&facts, "w").waiting);
        assert!(find(&facts, "u").unreported);
        assert!(find(&facts, "g").gone);
        assert_eq!(find(&facts, "f").activity, Some(ActivityState::Failed));
        let plain = find(&facts, "plain");
        assert!(plain.blocked.is_none() && !plain.waiting && !plain.unreported && !plain.gone);
        assert!(plain.activity.is_none());
    }

    #[test]
    fn the_ticket_comes_from_the_spawn_record_and_a_bad_record_is_none() {
        let store = tempfile::tempdir().unwrap();
        let root = store.path();
        owner(root, CLAUDE_OWNER_DIR, "t");
        owner(root, CLAUDE_OWNER_DIR, "bad");
        owner(root, CLAUDE_OWNER_DIR, "none");
        let record = SpawnRecord {
            ticket: Some("GUZ-9".to_string()),
            repo: Some("/Users/alice/repo".to_string()),
            lane: Some("/Users/alice/lane".to_string()),
            ..SpawnRecord::default()
        };
        touch(
            &spawn_record_path(root, "t"),
            &encode_spawn_record(&record).unwrap(),
        );
        touch(&spawn_record_path(root, "bad"), "{not json");
        let facts = worker_facts(root);
        assert_eq!(find(&facts, "t").ticket.as_deref(), Some("GUZ-9"));
        assert!(find(&facts, "bad").ticket.is_none());
        assert!(find(&facts, "none").ticket.is_none());
    }

    #[test]
    fn the_last_report_is_the_newest_logged_one_for_that_worker() {
        let store = tempfile::tempdir().unwrap();
        let root = store.path();
        owner(root, CLAUDE_OWNER_DIR, "A");
        owner(root, CLAUDE_OWNER_DIR, "B");
        let entry = |at: &str, text: &str| {
            encode_entry(&NewEntry {
                role: Role::Worker,
                at: at.to_string(),
                text: text.to_string(),
            })
            .unwrap()
        };
        let log = [
            entry("2026-01-01T00:00:00Z", "[worker A · question]\nwhy"),
            entry("2026-01-01T00:05:00Z", "[worker A · done]\nfinished"),
        ]
        .concat();
        touch(&root.join("log.jsonl"), &log);
        let facts = worker_facts(root);
        let last = find(&facts, "A").last_report.clone().unwrap();
        assert_eq!(
            (last.status.as_str(), last.at.as_str()),
            ("done", "2026-01-01T00:05:00Z")
        );
        assert!(find(&facts, "B").last_report.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_never_followed() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("outside");
        let record = SpawnRecord {
            ticket: Some("GUZ-9".to_string()),
            ..SpawnRecord::default()
        };
        touch(&target, &encode_spawn_record(&record).unwrap());
        let store = tempfile::tempdir().unwrap();
        let root = store.path();
        let link = |path: &Path| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            symlink(&target, path).unwrap();
        };
        link(&root.join(CLAUDE_OWNER_DIR).join("linked"));
        owner(root, CLAUDE_OWNER_DIR, "real");
        link(&spawn_record_path(root, "real"));
        link(&blocked_path(root, "real"));
        link(&activity_path(root, "real"));
        link(&waiting_path(root, "real"));
        link(&gone_path(root, "real"));
        let facts = worker_facts(root);
        assert_eq!(facts.len(), 1, "{facts:?}");
        let real = find(&facts, "real");
        assert!(real.ticket.is_none());
        assert!(real.blocked.is_none() && real.activity.is_none());
        assert!(!real.waiting && !real.gone);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_in_the_store_is_never_followed() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        owner(outside.path(), ".", "stranger");
        let record = SpawnRecord {
            ticket: Some("GUZ-9".to_string()),
            ..SpawnRecord::default()
        };
        touch(
            &outside.path().join("real"),
            &encode_spawn_record(&record).unwrap(),
        );
        let store = tempfile::tempdir().unwrap();
        let root = store.path();
        symlink(outside.path(), root.join(CLAUDE_OWNER_DIR)).unwrap();
        owner(root, CODEX_OWNER_DIR, "real");
        let spawn_dir = spawn_record_path(root, "real")
            .parent()
            .unwrap()
            .to_path_buf();
        symlink(outside.path(), &spawn_dir).unwrap();
        let facts = worker_facts(root);
        let ids: Vec<&str> = facts.iter().map(|facts| facts.id.as_str()).collect();
        assert_eq!(ids, ["real"], "{facts:?}");
        assert!(find(&facts, "real").ticket.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_log_gives_no_report() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        let log = encode_entry(&NewEntry {
            role: Role::Worker,
            at: "2026-01-01T00:05:00Z".to_string(),
            text: "[worker A · done]\nfinished".to_string(),
        })
        .unwrap();
        touch(&outside.path().join("log.jsonl"), &log);
        let store = tempfile::tempdir().unwrap();
        owner(store.path(), CLAUDE_OWNER_DIR, "A");
        symlink(
            outside.path().join("log.jsonl"),
            store.path().join("log.jsonl"),
        )
        .unwrap();
        let facts = worker_facts(store.path());
        assert!(find(&facts, "A").last_report.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_linked_marker_is_not_read() {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("private");
        touch(&target, "private text");
        let store = tempfile::tempdir().unwrap();
        let root = store.path();
        owner(root, CLAUDE_OWNER_DIR, "real");
        fs::create_dir_all(blocked_path(root, "real").parent().unwrap()).unwrap();
        fs::hard_link(&target, blocked_path(root, "real")).unwrap();
        let facts = worker_facts(root);
        assert!(find(&facts, "real").blocked.is_none());
    }
}
