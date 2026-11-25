use super::*;

impl CursorHost {
    pub(super) fn resume_all(&self) {
        for id in request_ids(&self.store_dir, self.dir(CURSOR_SESSION_DIR, "agy-session")) {
            self.resume(&id);
        }
    }

    fn resume(&self, id: &str) {
        let files = self.files(id);
        if files.owner.exists() && !self.agy {
            warn(format!(
                "{} session {id} not resumed: a running chat owns it",
                CURSOR_AGENT.name
            ));
            return;
        }
        let Some(record) = fs::read_to_string(&files.session)
            .ok()
            .and_then(|text| SessionRecord::parse(&text))
        else {
            warn(format!(
                "{} session {id} not resumed: its record is unreadable",
                CURSOR_AGENT.name
            ));
            return;
        };
        if let Resume::Skip(message) = recovery::resume(
            if self.agy { &AGY_AGENT } else { &CURSOR_AGENT },
            id,
            &record,
            &self.store_dir,
            true,
        ) {
            warn(message);
            return;
        }
        if self.agy {
            let Ok(_lock) = recovery::lock(&files.owner) else {
                return;
            };
            if files.owner.exists() && !crate::cursor_owner::reclaim_agy(&files.owner) {
                return;
            }
            if let Err(error) = journal::recover(&self.store_dir, id) {
                warn(format!(
                    "agy session {id} not resumed: could not report unfinished work: {error}"
                ));
                return;
            }
        }
        let Ok(exe) = std::env::current_exe() else {
            warn(format!(
                "{} session {id} not resumed: could not find the ralph executable",
                CURSOR_AGENT.name
            ));
            return;
        };
        if !record.cwd.is_dir() {
            let message = format!(
                "{} session {id} not resumed: its worktree {} is gone",
                if self.agy {
                    AGY_AGENT.name
                } else {
                    CURSOR_AGENT.name
                },
                record.cwd.display()
            );
            warn(message.clone());
            if self.agy {
                write_file(&files.error, &message).ok();
            }
            return;
        }
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
            scheduled: false,
        };
        if self.agy && !recovery::claim_saved(&files, &record) {
            return;
        }
        let started = if !self.agy && record.permissions.is_some() {
            let Some(mode) = record.permissions else {
                return;
            };
            PrintWorker::start(id, &request, &files, Some(&record), mode).map(CursorWorker::Print)
        } else {
            let launch = Launch::Resume {
                record,
                store: self.store_dir.clone(),
            };
            self.launch(id, &request, launch)
                .map(|worker| CursorWorker::Acp(Box::new(worker)))
        };
        match started {
            Ok(worker) => self.adopt_resumed(id, worker),
            Err(message) => {
                if self.agy {
                    recovery::remove_owned(&files.owner);
                }
                warn(message.clone());
                write_file(&files.error, &message).ok();
            }
        }
    }

    fn adopt_resumed(&self, id: &str, worker: CursorWorker) {
        let ready = ready_path(&self.store_dir, id);
        if !self.closed.load(Ordering::SeqCst) && write_file(&ready, "").is_ok() {
            self.workers().insert(id.to_string(), worker);
        } else {
            drop(worker);
            if self.agy {
                self.forget(id, false);
            }
            if !self.agy {
                fs::remove_file(ready).ok();
            }
        }
    }
}
