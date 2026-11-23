use super::read_optional;
use crate::chat::thread::thread_dir;
use ralph_core::chat_memory::{parse_log, parse_tree, tree_within, Entry, Tree};
use ralph_core::chat_thread::{ThreadMeta, ThreadName, THREADS_DIR, THREAD_FILE};
use std::fs;
use std::io;
use std::path::Path;

pub struct Peek {
    pub entries: Vec<Entry>,
    pub tree: Tree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInfo {
    pub name: String,
    pub parked: bool,
    pub merged: bool,
}

pub fn peek(dir: &Path) -> io::Result<Peek> {
    if !dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no chat store at {}", dir.display()),
        ));
    }
    let log = read_optional(&dir.join("log.jsonl"))?;
    let tree = read_optional(&dir.join("tree.jsonl"))?;
    let (entries, _) = parse_log(&log);
    let (tree, _) = parse_tree(&tree);
    let tree = tree_within(&tree, entries.len());
    Ok(Peek { entries, tree })
}

pub fn thread_exists(main: &Path, name: &ThreadName) -> bool {
    thread_dir(main, name).join(THREAD_FILE).is_file()
}

pub fn list_threads(main: &Path) -> io::Result<Vec<ThreadInfo>> {
    let items = match fs::read_dir(main.join(THREADS_DIR)) {
        Ok(items) => items,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut threads = Vec::new();
    for item in items {
        let item = item?;
        let Some(name) = item.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if ThreadName::parse(&name).is_err() {
            continue;
        }
        let Ok(bytes) = read_optional(&item.path().join(THREAD_FILE)) else {
            continue;
        };
        let Ok(meta) = serde_json::from_slice::<ThreadMeta>(&bytes) else {
            continue;
        };
        threads.push(ThreadInfo {
            name,
            parked: meta.parked.is_some(),
            merged: meta.merged.is_some(),
        });
    }
    threads.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(threads)
}
