use crate::chat::fork::read_meta;
use ralph_core::chat_thread::{ThreadMeta, ThreadName, THREADS_DIR};
use ralph_core::chat_thread_auto::{pending_closes, PendingClose};
use std::path::Path;

fn closed_threads(main: &Path) -> Vec<(ThreadName, ThreadMeta)> {
    let Ok(entries) = std::fs::read_dir(main.join(THREADS_DIR)) else {
        return Vec::new();
    };
    let mut threads: Vec<(ThreadName, ThreadMeta)> = entries
        .filter_map(|entry| ThreadName::parse(entry.ok()?.file_name().to_str()?).ok())
        .filter_map(|name| {
            let meta = read_meta(&main.join(THREADS_DIR).join(name.as_str())).ok()?;
            Some((name, meta))
        })
        .collect();
    threads.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    threads
}

pub fn pending(main: &Path) -> Vec<PendingClose> {
    pending_closes(&closed_threads(main))
}
