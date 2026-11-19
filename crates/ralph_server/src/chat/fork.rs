use crate::chat::memory::Store;
use ralph_core::chat_memory::{view, ViewSplit};
use ralph_core::chat_thread::{
    fork_of, thread_split, ThreadMeta, ThreadName, MAIN_PREFIX_LINES, THREAD_FILE,
};
use std::fs;
use std::io;
use std::path::Path;

pub struct Fork {
    pub main: Store,
    pub name: ThreadName,
    pub meta: ThreadMeta,
}

pub fn open_fork(store: &Path) -> Result<Option<Fork>, String> {
    let Some((main, name)) = fork_of(store) else {
        return Ok(None);
    };
    let meta = read_meta(store).map_err(|e| format!("thread {name}: {e}"))?;
    Ok(Some(Fork {
        main: Store::new(main),
        name,
        meta,
    }))
}

pub fn fork_view(fork: &Fork, own: ViewSplit) -> io::Result<ViewSplit> {
    let (entries, tree) = fork.main.load()?;
    let base = fork.meta.base.min(entries.len());
    let prefix = view(&entries[..base], &tree, MAIN_PREFIX_LINES);
    Ok(thread_split(prefix, fork.meta.base, &fork.name, own))
}

pub fn read_meta(dir: &Path) -> io::Result<ThreadMeta> {
    let text = fs::read_to_string(dir.join(THREAD_FILE))?;
    serde_json::from_str(&text).map_err(io::Error::other)
}
