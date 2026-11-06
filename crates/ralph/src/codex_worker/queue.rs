use super::*;
use crate::workers::headless::{drain_queued, queued_owner};
use ralph_core::worker::{QueuedFacts, QueuedWorker};

impl CodexHost {
    fn queued_facts(&self, id: &str) -> QueuedFacts {
        let worker = match locked(&self.workers).get(id) {
            None => QueuedWorker::Absent,
            Some(worker) if worker.exited() => QueuedWorker::Exited,
            Some(_) => QueuedWorker::Alive,
        };
        QueuedFacts {
            worker,
            starting: locked(&self.starting).contains_key(id),
            owner: queued_owner(&self.files(id).owner),
        }
    }

    pub(super) fn deliver_queued(
        &self,
        id: &str,
        dir: fn(&HandoffFiles) -> &PathBuf,
        deliver: fn(&mut CodexWorker, String, &str) -> Result<(), String>,
        lost: fn(&HandoffFiles, &Path),
    ) {
        let files = self.files(id);
        drain_queued(
            dir(&files),
            || self.queued_facts(id),
            |text, name| match locked(&self.workers).get_mut(id) {
                Some(worker) => deliver(worker, text, name),
                None => Ok(()),
            },
            |entry| lost(&files, entry),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::worker::lost_marker_name;

    fn queued(dir: &Path, name: &str) -> PathBuf {
        write_file(&dir.join(name), "hello").unwrap();
        dir.join(name)
    }

    #[test]
    fn a_prompt_with_no_owner_leaves_a_lost_marker() {
        let store = tempfile::tempdir().unwrap();
        let files = HandoffFiles::codex(store.path(), "A");
        let entry = queued(&files.prompt, "0001");
        CodexHost::new(store.path().to_path_buf()).poll();
        assert!(!entry.exists());
        assert!(files.prompt.join(lost_marker_name("0001")).exists());
    }

    #[test]
    fn a_prompt_this_chat_owns_but_has_not_adopted_stays_queued() {
        let store = tempfile::tempdir().unwrap();
        let files = HandoffFiles::codex(store.path(), "A");
        let host = CodexHost::new(store.path().to_path_buf());
        claim_owner(&files.owner).unwrap();
        let entry = queued(&files.prompt, "0001");
        host.poll();
        assert!(entry.exists());
        assert!(!files.prompt.join(lost_marker_name("0001")).exists());
    }

    #[test]
    fn an_interrupt_with_no_worker_reports_gone() {
        let store = tempfile::tempdir().unwrap();
        let files = HandoffFiles::codex(store.path(), "A");
        let entry = queued(&files.interrupt, "0001");
        CodexHost::new(store.path().to_path_buf()).poll();
        assert!(!entry.exists());
        assert_eq!(
            fs::read_to_string(files.interrupt_result.join("0001")).unwrap(),
            "gone"
        );
    }
}
