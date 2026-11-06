use super::*;
use ralph_core::worker::{
    lost_marker_name, queued_disposition, InterruptOutcome, OwnerRecord, Queued, QueuedFacts,
    QueuedOwner, QueuedWorker,
};

pub(super) fn mark_prompt_lost(_: &HandoffFiles, dir: &Path, name: &str) {
    fs::rename(dir.join(name), dir.join(lost_marker_name(name))).ok();
}

pub(super) fn report_interrupt_gone(files: &HandoffFiles, dir: &Path, name: &str) {
    write_interrupt_result(&files.interrupt_result, name, InterruptOutcome::Gone);
    fs::remove_file(dir.join(name)).ok();
}

impl CodexHost {
    fn queued_facts(&self, id: &str) -> QueuedFacts {
        let worker = match locked(&self.workers).get(id) {
            None => QueuedWorker::Absent,
            Some(worker) if worker.exited() => QueuedWorker::Exited,
            Some(_) => QueuedWorker::Alive,
        };
        let owner = match fs::read_to_string(&self.files(id).owner) {
            Err(_) => QueuedOwner::Missing,
            Ok(text) => match OwnerRecord::parse(&text) {
                Some(record) if record.chat_pid == std::process::id() => QueuedOwner::Mine,
                _ => QueuedOwner::Foreign,
            },
        };
        QueuedFacts {
            worker,
            starting: locked(&self.starting).contains_key(id),
            owner,
        }
    }

    pub(super) fn deliver_queued(
        &self,
        id: &str,
        dir: fn(&HandoffFiles) -> &PathBuf,
        deliver: fn(&mut CodexWorker, String, &str) -> Result<(), String>,
        lost: fn(&HandoffFiles, &Path, &str),
    ) {
        let files = self.files(id);
        for entry in queued_entries(dir(&files)) {
            let facts = self.queued_facts(id);
            let name = entry
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let parent = dir(&files);
            let delivered = match queued_disposition(&facts) {
                Queued::Hold => continue,
                Queued::Lost => Err(None),
                Queued::Deliver => match (
                    fs::read_to_string(&entry),
                    locked(&self.workers).get_mut(id),
                ) {
                    (Ok(text), Some(worker)) => deliver(worker, text, &name).map_err(Some),
                    _ => Ok(()),
                },
            };
            match delivered {
                Ok(()) => {
                    fs::remove_file(&entry).ok();
                }
                Err(message) => {
                    if let Some(message) = message {
                        warn(message);
                    }
                    lost(&files, parent, &name);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
