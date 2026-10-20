use crate::chat::index_files::{
    open_archive, save_stale, sweep_index_temps, update_index, FileSource,
};
use crate::warn::warn;
use ralph_core::chat_memory::{
    check_summary, commit_decision, encode_entry, encode_record, encode_usage, fallback_summary,
    log_tail, nap_source, nap_step, parse_log, parse_tree, pending, recall, retry_prompt, search,
    Archive, Commit, Entry, LogTail, NapStep, Node, NodeSlot, RangeRequest, Role, SearchRequest,
    Tree, Trigger, UsageRecord, MAX_NAP_ATTEMPTS,
};
use ralph_core::worker::{parse_pending_turn, pending_turn_marker, PENDING_TURN_FILE};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const RECALL_CHUNK_ENTRIES: usize = 256;

pub struct Store {
    dir: PathBuf,
}

pub(super) fn check_path(path: &Path) -> io::Result<()> {
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

pub(super) fn read_optional(path: &Path) -> io::Result<Vec<u8>> {
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

    /// Runs `call` over the indexed archive under a shared `LOCK`, then saves
    /// the indexes it had to extend or rebuild. A missing store reads as
    /// empty and nothing is created.
    fn with_archive<T>(
        &self,
        call: impl FnOnce(&mut Archive<FileSource>) -> io::Result<T>,
    ) -> io::Result<T> {
        let lock = if self.dir.is_dir() {
            let lock = self.lock_file()?;
            lock.lock_shared()?;
            Some(lock)
        } else {
            None
        };
        let mut archive = open_archive(&self.dir)?;
        let answer = call(&mut archive)?;
        for warning in archive.warnings() {
            warn(warning);
        }
        if lock.is_some() {
            if let Err(e) = save_stale(&self.dir, &archive) {
                warn(format!("chat memory index not saved: {e}"));
            }
        }
        Ok(answer)
    }

    pub fn read(&self, request: &RangeRequest) -> io::Result<Result<String, String>> {
        let page = self.with_archive(|archive| archive.read(request))?;
        Ok(page.map(|page| page.render()))
    }

    pub fn zoom(&self, node: Node) -> io::Result<Result<String, String>> {
        let lines = self.with_archive(|archive| archive.zoom(node))?;
        Ok(lines.map(|lines| lines.join("\n")))
    }

    pub fn recall(&self, pattern: &str) -> io::Result<Result<String, String>> {
        self.with_archive(|archive| {
            let mut entries = Vec::new();
            archive.scan_entries(RECALL_CHUNK_ENTRIES, |_, entry| entries.push(entry.clone()))?;
            Ok(recall(&entries, pattern))
        })
    }

    pub fn search(&self, request: &SearchRequest) -> io::Result<Result<String, String>> {
        self.with_archive(|archive| search(archive, request))
    }

    pub fn load_usage(&self) -> io::Result<String> {
        let bytes = read_optional(&self.dir.join("usage.jsonl"))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn open_lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.dir)?;
        self.lock_file()
    }

    fn lock_file(&self) -> io::Result<File> {
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
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)?;
        trim_torn_tail(&mut file)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        // The line is committed; `log.idx` is a cache the next reader rebuilds.
        if let Err(e) = update_index::<u64>(&self.dir) {
            warn(format!("chat log index not updated: {e}"));
        }
        // No save runs under the exclusive lock, so any index temp file left
        // here is from a killed save.
        if let Err(e) = sweep_index_temps(&self.dir) {
            warn(format!("chat index temp files not removed: {e}"));
        }
        Ok(())
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
                // The record is committed; `tree.idx` is a cache like `log.idx`.
                if let Err(e) = update_index::<NodeSlot>(&self.dir) {
                    warn(format!("chat tree index not updated: {e}"));
                }
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

const TAIL_PROBE_BYTES: u64 = 4096;

fn trim_torn_tail(file: &mut File) -> io::Result<()> {
    let len = file.metadata()?.len();
    let mut window = TAIL_PROBE_BYTES;
    loop {
        let start = len.saturating_sub(window);
        let mut chunk = vec![0; usize::try_from(len - start).map_err(io::Error::other)?];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;
        match log_tail(&chunk, start) {
            LogTail::Whole => return Ok(()),
            LogTail::Torn(at) => {
                warn(format!(
                    "chat log dropped a torn last line of {} bytes at byte {at}",
                    len - at
                ));
                file.set_len(at)?;
                return file.sync_all();
            }
            LogTail::NeedMore => window = window.saturating_mul(2),
        }
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
    use ralph_core::chat_memory::{
        frozen_block, parse_search_args, LogIndex, TreeIndex, FALLBACK_MARK, LOG_INDEX_FILE,
        TREE_INDEX_FILE,
    };

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

    fn stored_index(store: &Store) -> Option<LogIndex> {
        LogIndex::decode(&fs::read(store.dir().join(LOG_INDEX_FILE)).ok()?)
    }

    fn read_request(id: &str) -> RangeRequest {
        ralph_core::chat_memory::parse_tool_call("read", &serde_json::json!({ "id": id }))
            .map(|tool| match tool {
                ralph_core::chat_memory::Tool::Read(request) => request,
                _ => unreachable!(),
            })
            .unwrap()
    }

    #[test]
    fn append_keeps_the_log_index_current() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        store.append(Role::User, "hi").unwrap();
        assert_eq!(stored_index(&store).unwrap().count(), 1);
        store.append(Role::Assistant, "yo").unwrap();
        let index = stored_index(&store).unwrap();
        assert_eq!(index.count(), 2);
        let log = fs::read(store.dir().join("log.jsonl")).unwrap();
        assert_eq!(index.covered, log.len() as u64);
    }

    #[test]
    fn append_rebuilds_a_damaged_log_index() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        store.append(Role::User, "one").unwrap();
        fs::write(store.dir().join(LOG_INDEX_FILE), b"junk").unwrap();
        store.append(Role::User, "two").unwrap();
        assert_eq!(stored_index(&store).unwrap().count(), 2);
    }

    #[test]
    fn append_after_a_torn_line_keeps_the_new_entry_visible() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        store.append(Role::User, "zero").unwrap();
        let mut log = OpenOptions::new()
            .append(true)
            .open(store.dir().join("log.jsonl"))
            .unwrap();
        log.write_all(b"{\"role\":\"user\",\"at\":\"t\",\"te")
            .unwrap();
        store.append(Role::Assistant, "one").unwrap();
        let (entries, _) = store.load().unwrap();
        let texts: Vec<_> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["zero", "one"]);
        assert_eq!(stored_index(&store).unwrap().count(), 2);
        let page = store.read(&read_request("0-1")).unwrap().unwrap();
        assert!(page.contains("one"), "{page}");
    }

    #[test]
    fn readers_keep_entries_after_a_corrupt_line_visible() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        store.append(Role::User, "needle zero").unwrap();
        let mut log = OpenOptions::new()
            .append(true)
            .open(store.dir().join("log.jsonl"))
            .unwrap();
        log.write_all(b"not an entry needle\n").unwrap();
        store.append(Role::Assistant, "needle two").unwrap();
        store.append(Role::User, "needle three").unwrap();
        let page = store.read(&read_request("0-3")).unwrap().unwrap();
        assert!(
            page.contains("#1 · worker ·  · 15 chars\n[corrupt entry]"),
            "{page}"
        );
        assert!(
            page.contains("needle two") && page.contains("needle three"),
            "{page}"
        );
        let zoomed = store.zoom(Node::new(3, 4).unwrap()).unwrap().unwrap();
        assert!(zoomed.starts_with("#3 user "), "{zoomed}");
        let recalled = store.recall("needle").unwrap().unwrap();
        let hits: Vec<_> = recalled.lines().filter(|l| l.starts_with('#')).collect();
        assert_eq!(hits.len(), 3, "{recalled}");
        assert!(
            hits[0].starts_with("#3 ") && hits[2].starts_with("#0 "),
            "{recalled}"
        );
        let request = parse_search_args(&serde_json::json!({ "query": "needle corrupt" })).unwrap();
        let found = store.search(&request).unwrap().unwrap();
        let ids: Vec<_> = found.lines().filter(|l| l.starts_with('#')).collect();
        assert_eq!(ids.len(), 3, "{found}");
        assert!(!found.contains("#1 "), "{found}");
        assert_eq!(stored_index(&store).unwrap().count(), 4);
    }

    #[test]
    fn append_removes_orphaned_index_temps_and_keeps_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        store.append(Role::User, "one").unwrap();
        let index = store.dir().join(LOG_INDEX_FILE);
        let (_, orphan) = crate::chat::index_files::index_temp(store.dir(), &index)
            .unwrap()
            .keep()
            .unwrap();
        let unrelated = [".tmpAb3dE9", ".log.idx.abc.tmp", "notes.tmp"];
        for name in unrelated {
            fs::write(store.dir().join(name), "keep").unwrap();
        }
        assert!(orphan.exists());
        store.append(Role::User, "two").unwrap();
        assert!(!orphan.exists());
        for name in unrelated {
            assert!(store.dir().join(name).exists(), "{name}");
        }
        assert_eq!(stored_index(&store).unwrap().count(), 2);
    }

    #[test]
    fn reads_see_lines_the_index_has_not_caught_up_with() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        store.append(Role::User, "first").unwrap();
        let mut log = OpenOptions::new()
            .append(true)
            .open(store.dir().join("log.jsonl"))
            .unwrap();
        log.write_all(b"{\"role\":\"user\",\"at\":\"t\",\"text\":\"second\"}\n")
            .unwrap();
        assert_eq!(stored_index(&store).unwrap().count(), 1);
        let page = store.read(&read_request("0-9")).unwrap().unwrap();
        assert!(page.contains("first") && page.contains("second"));
        assert!(store
            .zoom(Node::new(1, 2).unwrap())
            .unwrap()
            .unwrap()
            .contains("second"));
        assert_eq!(stored_index(&store).unwrap().count(), 2);
    }

    #[test]
    fn reads_heal_a_damaged_log_index_that_still_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        for text in ["zero", "one", "two"] {
            store.append(Role::User, text).unwrap();
        }
        let mut index = stored_index(&store).unwrap();
        index.slots[1] += 2;
        fs::write(store.dir().join(LOG_INDEX_FILE), index.encode()).unwrap();
        let zoomed = store.zoom(Node::new(1, 2).unwrap()).unwrap().unwrap();
        assert!(zoomed.contains("one"), "{zoomed}");
        let page = store.read(&read_request("0-2")).unwrap().unwrap();
        assert!(page.contains("zero") && page.contains("two"), "{page}");
        let log = fs::read(store.dir().join("log.jsonl")).unwrap();
        let mut built = LogIndex::default();
        built.extend(&log);
        assert_eq!(stored_index(&store), Some(built));
    }

    #[test]
    fn reads_without_a_log_report_beyond_the_chat() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("s"));
        assert!(store.read(&read_request("0")).unwrap().is_err());
        assert!(store.zoom(Node::new(0, 1).unwrap()).unwrap().is_err());
        assert!(!store.dir().exists());
        fs::create_dir(store.dir()).unwrap();
        assert!(store.read(&read_request("0")).unwrap().is_err());
        let names: Vec<_> = fs::read_dir(store.dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["LOCK"]);
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

    #[test]
    fn nap_keeps_the_tree_index_current_and_zoom_reads_through_it() {
        let (_dir, store) = seeded(&[1], 4);
        let mut calls = 0;
        store
            .nap(|_| {
                calls += 1;
                Ok(format!("sum{calls}"))
            })
            .unwrap();
        let bytes = fs::read(store.dir().join(TREE_INDEX_FILE)).unwrap();
        let index = TreeIndex::decode(&bytes).unwrap();
        let tree = fs::read(store.dir().join("tree.jsonl")).unwrap();
        assert_eq!((index.count(), index.covered), (4, tree.len() as u64));
        let zoomed = store.zoom(Node::new(0, 4).unwrap()).unwrap().unwrap();
        assert!(
            zoomed.contains("sum2") && zoomed.contains("sum3"),
            "{zoomed}"
        );
    }

    #[test]
    fn reads_see_tree_records_without_a_usable_tree_index() {
        let (_dir, store) = seeded(&[], 4);
        let first = encode_record(Node::new(0, 2).unwrap(), "first").unwrap();
        let loser = encode_record(Node::new(0, 2).unwrap(), "loser").unwrap();
        fs::write(store.dir().join("tree.jsonl"), first + &loser).unwrap();
        fs::write(store.dir().join(TREE_INDEX_FILE), b"junk").unwrap();
        let zoomed = store.zoom(Node::new(0, 4).unwrap()).unwrap().unwrap();
        assert!(
            zoomed.contains("first") && !zoomed.contains("loser"),
            "{zoomed}"
        );
        let saved = fs::read(store.dir().join(TREE_INDEX_FILE)).unwrap();
        assert_eq!(TreeIndex::decode(&saved).unwrap().count(), 2);
        fs::remove_file(store.dir().join(TREE_INDEX_FILE)).unwrap();
        store.read(&read_request("0-3")).unwrap().unwrap();
        let again = fs::read(store.dir().join(TREE_INDEX_FILE)).unwrap();
        assert_eq!(again, saved);
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
