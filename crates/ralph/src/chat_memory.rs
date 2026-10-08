use crate::warn::warn;
use ralph_core::chat_memory::{parse_log, parse_tree, Entry, Tree};
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub struct Store {
    dir: PathBuf,
}

fn read_optional(path: &Path) -> io::Result<Vec<u8>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::other(format!(
            "chat store path is a symlink: {}",
            path.display()
        ))),
        #[cfg(unix)]
        Ok(metadata) if metadata.is_file() && metadata.nlink() > 1 => Err(io::Error::other(
            format!("chat store file has multiple links: {}", path.display()),
        )),
        Ok(_) => fs::read(path),
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
}
