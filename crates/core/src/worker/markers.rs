use super::{BLOCKED_DIR, READY_DIR, REPORTED_DIR, SENT_DIR, UNREPORTED_DIR, WAITING_DIR};
use std::path::{Path, PathBuf};

pub fn ready_path(store: &Path, id: &str) -> PathBuf {
    store.join(READY_DIR).join(id)
}

pub fn reported_path(store: &Path, id: &str) -> PathBuf {
    store.join(REPORTED_DIR).join(id)
}

pub fn blocked_path(store: &Path, id: &str) -> PathBuf {
    store.join(BLOCKED_DIR).join(id)
}

pub fn waiting_path(store: &Path, id: &str) -> PathBuf {
    store.join(WAITING_DIR).join(id)
}

pub fn unreported_path(store: &Path, id: &str) -> PathBuf {
    store.join(UNREPORTED_DIR).join(id)
}

pub fn sent_path(store: &Path, id: &str) -> PathBuf {
    store.join(SENT_DIR).join(id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerMarkers {
    pub ready: PathBuf,
    pub reported: PathBuf,
    pub blocked: PathBuf,
    pub waiting: PathBuf,
    pub unreported: PathBuf,
    pub sent: PathBuf,
}

impl WorkerMarkers {
    pub fn new(store: &Path, id: &str) -> Self {
        Self {
            ready: ready_path(store, id),
            reported: reported_path(store, id),
            blocked: blocked_path(store, id),
            waiting: waiting_path(store, id),
            unreported: unreported_path(store, id),
            sent: sent_path(store, id),
        }
    }
}

pub fn marker_paths(store: &Path, id: &str) -> [PathBuf; 6] {
    let WorkerMarkers {
        ready,
        reported,
        blocked,
        waiting,
        unreported,
        sent,
    } = WorkerMarkers::new(store, id);
    [ready, reported, blocked, waiting, unreported, sent]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_path_is_per_worker_in_the_store() {
        assert_eq!(
            ready_path(Path::new("/s"), "A"),
            PathBuf::from("/s/ready/A")
        );
    }

    #[test]
    fn reported_path_is_per_worker_in_the_store() {
        assert_eq!(
            reported_path(Path::new("/s"), "A"),
            PathBuf::from("/s/reported/A")
        );
    }

    #[test]
    fn sent_path_is_per_worker_in_the_store() {
        assert_eq!(sent_path(Path::new("/s"), "A"), PathBuf::from("/s/sent/A"));
    }

    #[test]
    fn marker_paths_cover_every_marker_dir_for_the_worker() {
        assert_eq!(
            marker_paths(Path::new("/s"), "A"),
            [
                PathBuf::from("/s/ready/A"),
                PathBuf::from("/s/reported/A"),
                PathBuf::from("/s/blocked/A"),
                PathBuf::from("/s/waiting/A"),
                PathBuf::from("/s/unreported/A"),
                PathBuf::from("/s/sent/A"),
            ]
        );
    }
}
