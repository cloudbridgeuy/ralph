use crate::warn::warn;
use ralph_core::chat_memory::{
    check_summary, encode_record, nap_prompt, parse_log, parse_tree, pending, retry_prompt, Entry,
    Node, Tree, MAX_NAP_ATTEMPTS,
};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
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

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn load(&self) -> io::Result<(Vec<Entry>, Tree)> {
        let log = read_optional(&self.dir.join("log.jsonl"))?;
        let tree = read_optional(&self.dir.join("tree.jsonl"))?;
        let (entries, log_warnings) = parse_log(&log);
        let (tree, tree_warnings) = parse_tree(&tree);
        for warning in log_warnings.iter().chain(&tree_warnings) {
            warn(warning);
        }
        Ok((entries, tree))
    }

    fn lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join("LOCK");
        check_path(&path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        lock.lock()?;
        Ok(lock)
    }

    fn put(&self, node: Node, text: &str) -> io::Result<()> {
        let line = encode_record(node, text).map_err(io::Error::other)?;
        let path = self.dir.join("tree.jsonl");
        check_path(&path)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()
    }

    pub fn nap(&self, mut curate: impl FnMut(&str) -> Result<String, String>) -> io::Result<()> {
        let _lock = self.lock()?;
        let (entries, mut tree) = self.load()?;
        for node in pending(&entries, &tree) {
            let base = match nap_prompt(&entries, &tree, node) {
                Ok(prompt) => prompt,
                Err(e) => {
                    warn(format!("chat memory skipped #{}: {e}", node.id()));
                    continue;
                }
            };
            let mut prompt = base.clone();
            for _ in 0..MAX_NAP_ATTEMPTS {
                let reply = match curate(&prompt) {
                    Ok(reply) => reply,
                    Err(e) => {
                        warn(format!("chat memory curator failed on #{}: {e}", node.id()));
                        break;
                    }
                };
                match check_summary(&reply) {
                    Ok(summary) => {
                        self.put(node, &summary)?;
                        tree.insert(node, summary);
                        break;
                    }
                    Err(e) => prompt = retry_prompt(&base, &e),
                }
            }
            if !tree.contains_key(&node) {
                warn(format!("chat memory left #{} pending", node.id()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_files_load_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (entries, tree) = Store::new(dir.path().join("none")).load().unwrap();
        assert!(entries.is_empty() && tree.is_empty());
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
    fn nap_gives_up_after_max_attempts_and_skips_parent() {
        let (_dir, store) = seeded(&[0], 2);
        let mut calls = 0;
        store
            .nap(|_| {
                calls += 1;
                Ok("z".repeat(600))
            })
            .unwrap();
        assert_eq!(calls, MAX_NAP_ATTEMPTS);
        let (_, tree) = store.load().unwrap();
        assert!(tree.is_empty());
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
