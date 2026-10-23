use super::*;

impl CodexHost {
    pub(super) fn resume_all(&self) {
        for id in request_ids(&self.store_dir, CODEX_SESSION_DIR) {
            self.resume(&id);
        }
    }

    fn resume_failed(&self, files: &HandoffFiles, message: &str) {
        warn(message);
        write_file(&files.error, message).ok();
    }

    fn resume(&self, id: &str) {
        let files = self.files(id);
        let Some(record) = fs::read_to_string(&files.session)
            .ok()
            .and_then(|text| SessionRecord::parse(&text))
        else {
            return self.resume_failed(
                &files,
                &format!("codex session {id} not resumed: its record is unreadable"),
            );
        };
        let facts = ResumeFacts {
            owned: files.owner.exists(),
            worktree_exists: record.cwd.is_dir(),
        };
        if let Resume::Skip(message) = codex_resume_decision(id, &record, &self.store_dir, facts) {
            if facts.owned {
                warn(message);
            } else {
                self.resume_failed(&files, &message);
            }
            return;
        }
        match claim_owner(&files.owner) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return warn(format!(
                    "codex session {id} not resumed: a running chat owns it"
                ));
            }
            Err(e) => {
                return self.resume_failed(
                    &files,
                    &format!("codex session {id} not resumed: could not claim it: {e}"),
                );
            }
        }
        let Ok(exe) = std::env::current_exe() else {
            fs::remove_file(&files.owner).ok();
            return self.resume_failed(
                &files,
                &format!("codex session {id} not resumed: could not find the ralph executable"),
            );
        };
        let request = SpawnRequest {
            worktree: record.cwd.clone(),
            exe,
            inbox: record.inbox.clone(),
            task: String::new(),
            at: now_secs(),
            snapshots: snapshots_enabled(
                AppConfig::load().ok().map(|config| config.changes.enabled),
            ),
            model: record.model.clone(),
        };
        match self.launch(id, &request, Launch::Resume(record)) {
            Ok(worker) => self.adopt_resumed(id, worker),
            Err(message) => {
                fs::remove_file(&files.owner).ok();
                self.resume_failed(
                    &files,
                    &format!("codex session {id} not resumed: {message}"),
                );
            }
        }
    }

    fn adopt_resumed(&self, id: &str, worker: CodexWorker) {
        let ready = ready_path(&self.store_dir, id);
        if !self.closed.load(Ordering::SeqCst) && write_file(&ready, "").is_ok() {
            locked(&self.workers).insert(id.to_string(), worker);
            warn(format!("codex session {id} resumed"));
        } else {
            drop(worker);
            fs::remove_file(ready).ok();
        }
    }
}
