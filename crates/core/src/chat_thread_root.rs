use crate::chat_thread::{ThreadName, THREADS_DIR};
use std::path::{Path, PathBuf};

pub fn fork_of(store: &Path) -> Option<(PathBuf, ThreadName)> {
    let threads = store.parent()?;
    if threads.file_name()? != THREADS_DIR {
        return None;
    }
    let name = ThreadName::parse(store.file_name()?.to_str()?).ok()?;
    Some((threads.parent()?.to_path_buf(), name))
}

pub fn config_root(store: &Path) -> PathBuf {
    fork_of(store).map_or_else(|| store.to_path_buf(), |(main, _)| main)
}
