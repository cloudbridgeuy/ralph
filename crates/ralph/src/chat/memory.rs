use crate::warn::warn;
use ralph_core::chat_memory::{
    check_summary, commit_decision, encode_entry, encode_record, encode_usage, fallback_summary,
    nap_source, nap_step, parse_log, parse_tree, pending, retry_prompt, Commit, Entry, NapStep,
    Node, Role, Tree, Trigger, UsageRecord, MAX_NAP_ATTEMPTS,
};
use ralph_core::worker::{parse_pending_turn, pending_turn_marker, PENDING_TURN_FILE};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub struct Store {
    dir: PathBuf,
}

fn check_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::other(format!(
            "chat store path is a symlink: {}",
            path.display()
        ))),
        #[cfg(unix)]
        Ok(metadata) if metadata.is_file() && metadata.nlink() > 1 => Err(io::Error::other(
            format!("chat store file has multiple links: {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn read_optional(path: &Path) -> io::Result<Vec<u8>> {
    check_path(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

fn must_wait(attempt: Result<(), TryLockError>) -> io::Result<bool> {
    match attempt {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn load(&self) -> io::Result<(Vec<Entry>, Tree)> {
        let log = read_optional(&self.dir.join("log.jsonl"))?;
        let (entries, warnings) = parse_log(&log);
        for warning in &warnings {
            warn(warning);
        }
        Ok((entries, self.load_tree()?))
    }

    fn load_tree(&self) -> io::Result<Tree> {
        let bytes = read_optional(&self.dir.join("tree.jsonl"))?;
        let (tree, warnings) = parse_tree(&bytes);
        for warning in &warnings {
            warn(warning);
        }
        Ok(tree)
    }

    pub fn load_usage(&self) -> io::Result<String> {
        let bytes = read_optional(&self.dir.join("usage.jsonl"))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn open_lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join("LOCK");
        check_path(&path)?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
    }

    fn lock(&self) -> io::Result<File> {
        let lock = self.open_lock()?;
        lock.lock()?;
        Ok(lock)
    }

    /// `NAP` serializes naps for the whole run, so two naps never pay twice for
    /// the same node. It is separate from `LOCK`, which turns wait on.
    fn open_nap_lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join("NAP");
        check_path(&path)?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
    }

    pub fn load_settled<W>(&self, on_wait: impl FnOnce() -> W) -> io::Result<(Vec<Entry>, Tree)> {
        let lock = self.open_lock()?;
        if must_wait(lock.try_lock_shared())? {
            let _waiting = on_wait();
            lock.lock_shared()?;
        }
        self.load()
    }

    /// Shared try-lock probe on `NAP`: reports true while a nap runs, so another
    /// `ralph chat` on the same store also shows NAP.
    pub fn napping(&self) -> bool {
        self.open_nap_lock()
            .is_ok_and(|lock| matches!(lock.try_lock_shared(), Err(TryLockError::WouldBlock)))
    }

    pub fn append(&self, role: Role, text: &str) -> io::Result<()> {
        let _lock = self.lock()?;
        let entry = Entry {
            role,
            at: chrono::Utc::now().to_rfc3339(),
            text: text.to_string(),
        };
        let line = encode_entry(&entry).map_err(io::Error::other)?;
        let path = self.dir.join("log.jsonl");
        check_path(&path)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()
    }

    fn put(&self, node: Node, text: &str) -> io::Result<()> {
        let line = encode_record(node, text).map_err(io::Error::other)?;
        let path = self.dir.join("tree.jsonl");
        check_path(&path)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()
    }

    pub fn append_usage(&self, record: &UsageRecord) -> io::Result<()> {
        let line = encode_usage(record).map_err(io::Error::other)?;
        let path = self.dir.join("usage.jsonl");
        check_path(&path)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(line.as_bytes())
    }

    pub fn set_pending_turn(&self, trigger: Trigger) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(PENDING_TURN_FILE);
        check_path(&path)?;
        fs::write(path, pending_turn_marker(trigger))
    }

    pub fn clear_pending_turn(&self) {
        // Best effort: a missing marker already means no turn is in flight.
        let _ = fs::remove_file(self.dir.join(PENDING_TURN_FILE));
    }

    pub fn pending_turn(&self) -> io::Result<Option<Trigger>> {
        let marker = read_optional(&self.dir.join(PENDING_TURN_FILE))?;
        Ok(parse_pending_turn(&String::from_utf8_lossy(&marker)))
    }

    pub fn write_mcp_config(&self, config: &serde_json::Value) -> io::Result<PathBuf> {
        let path = self.dir.join("mcp.json");
        check_path(&path)?;
        let mut options = OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&path)?;
        file.write_all(config.to_string().as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        Ok(path)
    }

    /// Holds `LOCK` only to snapshot and to commit; curator calls run unlocked.
    pub fn nap(&self, mut curate: impl FnMut(&str) -> Result<String, String>) -> io::Result<()> {
        let nap_lock = self.open_nap_lock()?;
        nap_lock.lock()?;
        let (entries, mut tree) = self.load_settled(|| ())?;
        for node in pending(&entries, &tree) {
            let base = match nap_step(&entries, &tree, node) {
                NapStep::Done => continue,
                NapStep::Blocked(e) => {
                    warn(format!("chat memory skipped #{}: {e}", node.id()));
                    continue;
                }
                NapStep::Ask(prompt) => prompt,
            };
            match summarize(&entries, &tree, node, base, &mut curate) {
                Some(summary) => tree = self.commit(node, &summary)?,
                None => warn(format!("chat memory left #{} pending", node.id())),
            }
        }
        Ok(())
    }

    /// Re-reads the tree under the exclusive `LOCK` and writes the summary only
    /// when no other nap committed the node first. Returns the committed tree.
    fn commit(&self, node: Node, summary: &str) -> io::Result<Tree> {
        let _lock = self.lock()?;
        let mut tree = self.load_tree()?;
        match commit_decision(&tree, node) {
            Commit::Write => {
                self.put(node, summary)?;
                tree.insert(node, summary.to_string());
            }
            Commit::Taken => warn(format!(
                "chat memory kept the first summary for #{}",
                node.id()
            )),
        }
        Ok(tree)
    }
}

/// Runs the curator with `check_summary` retries. Returns the summary, the raw
/// fallback after `MAX_NAP_ATTEMPTS` rejections, or `None` on a curator error.
fn summarize(
    entries: &[Entry],
    tree: &Tree,
    node: Node,
    base: String,
    curate: &mut impl FnMut(&str) -> Result<String, String>,
) -> Option<String> {
    let mut prompt = base.clone();
    for _ in 0..MAX_NAP_ATTEMPTS {
        let reply = match curate(&prompt) {
            Ok(reply) => reply,
            Err(e) => {
                warn(format!("chat memory curator failed on #{}: {e}", node.id()));
                return None;
            }
        };
        match check_summary(&reply) {
            Ok(summary) => return Some(summary),
            Err(e) => prompt = retry_prompt(&base, &e),
        }
    }
    let source = nap_source(entries, tree, node).ok()?;
    warn(format!(
        "chat memory stored raw fallback for #{}",
        node.id()
    ));
    Some(fallback_summary(&source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_memory::{frozen_block, FALLBACK_MARK};

    #[test]
    fn missing_files_load_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (entries, tree) = Store::new(dir.path().join("none")).load().unwrap();
        assert!(entries.is_empty() && tree.is_empty());
    }

    #[test]
    fn must_wait_only_when_lock_is_held() {
        assert!(!must_wait(Ok(())).unwrap());
        assert!(must_wait(Err(TryLockError::WouldBlock)).unwrap());
        assert!(must_wait(Err(TryLockError::Error(io::Error::other("x")))).is_err());
    }

    #[test]
    fn napping_is_true_only_while_the_nap_lock_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        assert!(!store.napping());
        let store_lock = store.lock().unwrap();
        assert!(!store.napping());
        drop(store_lock);
        let held = store.open_nap_lock().unwrap();
        held.lock().unwrap();
        assert!(store.napping());
        drop(held);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while store.napping() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!store.napping());
    }

    #[test]
    fn napping_probe_does_not_block_settled_loads() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        let probe = store.open_lock().unwrap();
        probe.lock_shared().unwrap();
        let mut waited = false;
        store.load_settled(|| waited = true).unwrap();
        assert!(!waited);
    }

    #[test]
    fn append_round_trips_through_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("new"));
        store.append(Role::User, "hi").unwrap();
        store.append(Role::Assistant, "yo").unwrap();
        let (entries, _) = store.load_settled(|| ()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].role, Role::Assistant);
        assert_eq!(entries[1].text, "yo");
    }

    #[test]
    fn loads_log_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("log.jsonl"),
            "{\"role\":\"user\",\"at\":\"t\",\"text\":\"hi\"}\n",
        )
        .unwrap();
        let (entries, _) = Store::new(dir.path().to_path_buf()).load().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "hi");
    }

    #[test]
    fn symlinked_and_hardlinked_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::write(&real, "").unwrap();
        let store = Store::new(dir.path().to_path_buf());
        std::os::unix::fs::symlink(&real, dir.path().join("log.jsonl")).unwrap();
        assert!(store.load().is_err());
        fs::remove_file(dir.path().join("log.jsonl")).unwrap();
        fs::hard_link(&real, dir.path().join("log.jsonl")).unwrap();
        assert!(store.load().is_err());
    }

    fn seeded(long_at: &[usize], total: usize) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let log: String = (0..total)
            .map(|i| {
                let text = if long_at.contains(&i) {
                    "y".repeat(600)
                } else {
                    format!("m{i}")
                };
                format!("{{\"role\":\"user\",\"at\":\"t\",\"text\":\"{text}\"}}\n")
            })
            .collect();
        fs::write(dir.path().join("log.jsonl"), log).unwrap();
        let store = Store::new(dir.path().to_path_buf());
        (dir, store)
    }

    #[test]
    fn nap_builds_long_leaf_and_parents_then_is_idempotent() {
        let (_dir, store) = seeded(&[1], 4);
        let mut calls = 0;
        store
            .nap(|_| {
                calls += 1;
                Ok(format!("sum{calls}"))
            })
            .unwrap();
        assert_eq!(calls, 4);
        let (_, tree) = store.load().unwrap();
        assert_eq!(tree.len(), 4);
        let mut again = 0;
        store
            .nap(|_| {
                again += 1;
                Ok("x".into())
            })
            .unwrap();
        assert_eq!(again, 0);
    }

    fn frees_soon(lock: &std::fs::File) -> bool {
        (0..200).any(|_| {
            lock.try_lock().is_ok() || {
                std::thread::sleep(std::time::Duration::from_millis(10));
                false
            }
        })
    }

    #[test]
    fn nap_curator_runs_without_the_store_lock() {
        let (_dir, store) = seeded(&[0], 2);
        let mut free = Vec::new();
        store
            .nap(|_| {
                let probe = store.open_lock().unwrap();
                free.push(frees_soon(&probe));
                Ok("sum".into())
            })
            .unwrap();
        assert_eq!(free, [true, true]);
        assert_eq!(store.load().unwrap().1.len(), 2);
    }

    #[test]
    fn nap_gives_up_after_max_attempts_and_stores_fallback() {
        let (_dir, store) = seeded(&[0], 2);
        let mut calls = 0;
        store
            .nap(|_| {
                calls += 1;
                Ok("z".repeat(600))
            })
            .unwrap();
        assert_eq!(calls, MAX_NAP_ATTEMPTS * 2);
        let (_, tree) = store.load().unwrap();
        assert_eq!(tree.len(), 2);
        let leaf = &tree[&Node::new(0, 1).unwrap()];
        assert!(leaf.starts_with(FALLBACK_MARK) && leaf.len() <= 512);
        assert!(tree[&Node::new(0, 2).unwrap()].starts_with(FALLBACK_MARK));
    }

    #[test]
    fn nap_curator_error_leaves_node_pending_without_fallback() {
        let (_dir, store) = seeded(&[0], 1);
        store.nap(|_| Err("down".into())).unwrap();
        let (_, tree) = store.load().unwrap();
        assert!(tree.is_empty());
    }

    #[test]
    fn nap_leaves_pinned_frozen_block_byte_identical() {
        let (_dir, store) = seeded(&[0, 1, 2, 3], 4);
        let (entries, tree) = store.load().unwrap();
        let before = frozen_block(&entries, &tree, 200, 4, None);
        let mut calls = 0;
        store
            .nap(|_| {
                calls += 1;
                Ok(format!("sum{calls}"))
            })
            .unwrap();
        let (entries_after, tree_after) = store.load().unwrap();
        assert_eq!(entries_after.len(), entries.len());
        assert!(tree.is_empty() && tree_after.len() == 7);
        let rebuilt = frozen_block(&entries_after, &tree_after, 200, 4, None);
        assert_ne!(rebuilt.lines, before.lines);
        let pinned = frozen_block(&entries_after, &tree_after, 200, 4, Some(before.clone()));
        assert_eq!(pinned.lines.join("\n"), before.lines.join("\n"));
        assert_eq!(pinned, before);
    }

    #[test]
    fn nap_recovers_on_retry_with_error_in_prompt() {
        let (_dir, store) = seeded(&[0], 1);
        let mut prompts = Vec::new();
        store
            .nap(|prompt| {
                prompts.push(prompt.to_string());
                Ok(if prompts.len() == 1 {
                    "a\nb".to_string()
                } else {
                    "fine".to_string()
                })
            })
            .unwrap();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[1].contains("A summary is one line"));
        let (_, tree) = store.load().unwrap();
        assert_eq!(tree.values().collect::<Vec<_>>(), ["fine"]);
    }

    #[test]
    fn nap_curator_error_does_not_abort_other_nodes() {
        let (_dir, store) = seeded(&[0, 2], 4);
        let mut calls = 0;
        store
            .nap(|_| {
                calls += 1;
                if calls == 1 {
                    Err("boom".into())
                } else {
                    Ok("ok".into())
                }
            })
            .unwrap();
        let (_, tree) = store.load().unwrap();
        assert_eq!(tree.len(), 2);
    }
}
