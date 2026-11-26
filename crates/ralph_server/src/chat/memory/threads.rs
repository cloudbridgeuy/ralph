use super::Store;
use crate::chat::fork::read_meta;
use crate::chat::thread::thread_dir;
use crate::worker_mcp::Inbox;
use crate::workers::{canonical_store, running_workers};
use ralph_core::chat_thread::{ThreadMeta, ThreadName, THREADS_DIR};
use ralph_core::chat_thread_log::{parse_tombstones, Tombstone, DROPPED_FILE};
use ralph_core::chat_thread_table::{sort_rows, ThreadRow, ThreadState};
use ralph_core::worker::INBOX_DIR;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadsSnapshot {
    pub rows: Vec<ThreadRow>,
    pub head_messages: usize,
    pub tombstones: Vec<Tombstone>,
}

pub fn thread_rows(store: &Path) -> io::Result<ThreadsSnapshot> {
    let head_messages = Store::new(store.to_path_buf()).log_len()?;
    let mut rows = Vec::new();
    for (name, meta) in live_threads(store)? {
        rows.push(row_of(store, &name, &meta)?);
    }
    sort_rows(&mut rows);
    Ok(ThreadsSnapshot {
        rows,
        head_messages,
        tombstones: tombstones(store),
    })
}

fn live_threads(store: &Path) -> io::Result<Vec<(ThreadName, ThreadMeta)>> {
    let entries = match fs::read_dir(store.join(THREADS_DIR)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries?,
    };
    Ok(entries
        .filter_map(|entry| ThreadName::parse(entry.ok()?.file_name().to_str()?).ok())
        .filter_map(|name| {
            let meta = read_meta(&thread_dir(store, &name)).ok()?;
            Some((name, meta))
        })
        .collect())
}

fn row_of(store: &Path, name: &ThreadName, meta: &ThreadMeta) -> io::Result<ThreadRow> {
    let dir = thread_dir(store, name);
    Ok(ThreadRow {
        name: name.to_string(),
        state: state_of(meta),
        base: meta.base,
        created: meta.created.clone(),
        active: log_mtime(&dir),
        messages: Store::new(dir.clone()).log_len()?,
        workers: running_workers(&canonical_store(dir.clone())).len(),
        unread: Inbox::new(dir.join(INBOX_DIR)).unread_count()?,
    })
}

fn state_of(meta: &ThreadMeta) -> ThreadState {
    match (&meta.merged, &meta.parked) {
        (Some(merged), _) => ThreadState::Merged(merged.at.clone()),
        (None, Some(parked)) => ThreadState::Parked(parked.at.clone()),
        (None, None) => ThreadState::Open,
    }
}

fn log_mtime(dir: &Path) -> Option<String> {
    let modified = fs::metadata(dir.join("log.jsonl")).ok()?.modified().ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339())
}

fn tombstones(store: &Path) -> Vec<Tombstone> {
    let text = fs::read_to_string(store.join(THREADS_DIR).join(DROPPED_FILE)).unwrap_or_default();
    parse_tombstones(&text)
}

#[cfg(test)]
mod tests;
