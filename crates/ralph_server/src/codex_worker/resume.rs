use super::*;

pub(crate) struct Resumed<'a> {
    pub(crate) agent: &'a str,
    pub(crate) id: &'a str,
    pub(crate) store: &'a Path,
    pub(crate) files: &'a HandoffFiles,
}

impl Resumed<'_> {
    fn not_resumed(&self, why: &str) -> String {
        format!("{} session {} not resumed: {why}", self.agent, self.id)
    }

    fn failed(&self, why: &str) {
        let message = self.not_resumed(why);
        warn(&message);
        write_file(&self.files.error, &message).ok();
    }

    pub(crate) fn launch<W>(
        &self,
        launch: impl FnOnce(SessionRecord, SpawnRequest) -> Result<W, String>,
    ) -> Option<W> {
        let files = self.files;
        let Some(record) = fs::read_to_string(&files.session)
            .ok()
            .and_then(|text| SessionRecord::parse(&text))
        else {
            self.failed("its record is unreadable");
            return None;
        };
        let facts = ResumeFacts {
            owned: files.owner.exists(),
            worktree_exists: record.cwd.is_dir(),
        };
        if let Resume::Skip(message) =
            resume_decision_for(self.agent, self.id, &record, self.store, facts)
        {
            warn(&message);
            if !facts.owned {
                write_file(&files.error, &message).ok();
            }
            return None;
        }
        match claim_owner(&files.owner) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                warn(self.not_resumed("a running chat owns it"));
                return None;
            }
            Err(e) => {
                self.failed(&format!("could not claim it: {e}"));
                return None;
            }
        }
        let Ok(exe) = std::env::current_exe() else {
            fs::remove_file(&files.owner).ok();
            self.failed("could not find the ralph executable");
            return None;
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
        launch(record, request)
            .inspect_err(|message| {
                fs::remove_file(&files.owner).ok();
                self.failed(message);
            })
            .ok()
    }

    pub(crate) fn adopt(&self, closed: &AtomicBool) -> bool {
        let ready = ready_path(self.store, self.id);
        let adopted = !closed.load(Ordering::SeqCst) && write_file(&ready, "").is_ok();
        if adopted {
            fs::remove_file(&self.files.error).ok();
            notice(format!("{} session {} resumed", self.agent, self.id));
        } else {
            fs::remove_file(ready).ok();
        }
        adopted
    }
}

impl CodexHost {
    pub(super) fn resume_all(&self) {
        for id in request_ids(&self.store_dir, CODEX_SESSION_DIR) {
            let files = self.files(&id);
            let resumed = Resumed {
                agent: HarnessId::Codex.as_str(),
                id: &id,
                store: &self.store_dir,
                files: &files,
            };
            let worker = resumed
                .launch(|record, request| self.launch(&id, &request, Launch::Resume(record)));
            if let Some(worker) = worker {
                if resumed.adopt(&self.closed) {
                    locked(&self.workers).insert(id.clone(), worker);
                }
            }
        }
    }
}
