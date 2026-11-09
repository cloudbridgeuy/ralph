use super::*;
use crate::workers::headless::drain_queued;
use ralph_core::worker::{QueuedFacts, QueuedOwner, QueuedWorker};

fn queued_worker(exited: Option<bool>) -> QueuedWorker {
    match exited {
        None => QueuedWorker::Absent,
        Some(true) => QueuedWorker::Exited,
        Some(false) => QueuedWorker::Alive,
    }
}

fn deliver_to(
    worker: Option<&mut CursorWorker>,
    deliver: fn(&mut CursorWorker, String, &str) -> Result<(), String>,
    text: String,
    name: &str,
) -> Result<(), String> {
    match worker {
        Some(worker) => deliver(worker, text, name),
        None => Err(format!("worker has no running {} agent", CURSOR_AGENT.name)),
    }
}

impl CursorHost {
    fn queued_facts(&self, id: &str) -> QueuedFacts {
        QueuedFacts {
            worker: queued_worker(self.workers().get(id).map(CursorWorker::exited)),
            starting: locked(&self.starting).contains_key(id),
            owner: QueuedOwner::Missing,
        }
    }

    pub(super) fn deliver_queued(
        &self,
        id: &str,
        dir: fn(&HandoffFiles) -> &PathBuf,
        deliver: fn(&mut CursorWorker, String, &str) -> Result<(), String>,
        lost: fn(&HandoffFiles, &Path),
    ) {
        let files = self.files(id);
        drain_queued(
            dir(&files),
            || self.queued_facts(id),
            |text, name| deliver_to(self.workers().get_mut(id), deliver, text, name),
            |entry| lost(&files, entry),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor_owner::claim_owner;
    use ralph_core::worker::lost_marker_name;

    fn host(store: &Path) -> CursorHost {
        CursorHost {
            store_dir: store.to_path_buf(),
            workers: Mutex::new(HashMap::new()),
            starting: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        }
    }

    fn queued(dir: &Path, name: &str) -> PathBuf {
        write_file(&dir.join(name), "hello").unwrap();
        dir.join(name)
    }

    #[test]
    fn a_worker_gone_at_delivery_is_an_error() {
        assert!(deliver_to(None, |_, _, _| Ok(()), "hello".to_string(), "0001").is_err());
    }

    #[test]
    fn an_exited_worker_is_queued_as_exited() {
        assert_eq!(queued_worker(Some(true)), QueuedWorker::Exited);
        assert_eq!(queued_worker(Some(false)), QueuedWorker::Alive);
        assert_eq!(queued_worker(None), QueuedWorker::Absent);
    }

    #[test]
    fn a_prompt_with_no_worker_leaves_a_lost_marker() {
        let store = tempfile::tempdir().unwrap();
        let files = HandoffFiles::cursor(store.path(), "A");
        let entry = queued(&files.prompt, "0001");
        host(store.path()).poll();
        assert!(!entry.exists());
        assert!(files.prompt.join(lost_marker_name("0001")).exists());
    }

    #[test]
    fn a_prompt_with_a_stale_owner_and_no_worker_leaves_a_lost_marker() {
        let store = tempfile::tempdir().unwrap();
        let files = HandoffFiles::cursor(store.path(), "A");
        claim_owner(&files.owner).unwrap();
        let entry = queued(&files.prompt, "0001");
        host(store.path()).poll();
        assert!(!entry.exists());
        assert!(files.prompt.join(lost_marker_name("0001")).exists());
    }

    #[test]
    fn a_prompt_while_starting_stays_queued() {
        let store = tempfile::tempdir().unwrap();
        let files = HandoffFiles::cursor(store.path(), "A");
        let host = host(store.path());
        locked(&host.starting).insert("A".to_string(), 0);
        let entry = queued(&files.prompt, "0001");
        host.poll();
        assert!(entry.exists());
        assert!(!files.prompt.join(lost_marker_name("0001")).exists());
    }

    #[test]
    fn an_interrupt_with_no_worker_reports_gone() {
        let store = tempfile::tempdir().unwrap();
        let files = HandoffFiles::cursor(store.path(), "A");
        let entry = queued(&files.interrupt, "0001");
        host(store.path()).poll();
        assert!(!entry.exists());
        assert_eq!(
            fs::read_to_string(files.interrupt_result.join("0001"))
                .ok()
                .as_deref(),
            Some("gone")
        );
    }
}
