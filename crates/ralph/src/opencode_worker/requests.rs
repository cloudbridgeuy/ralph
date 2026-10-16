use super::{post_prompt, OpencodeHost, Session, Target};
use crate::acp_worker::{now_secs, write_file};
use crate::warn::warn;
use ralph_core::worker::{
    answer_decision, create_session_body, opencode_session_id, permission_reply_body,
    permission_reply_path, request_stale, session_missing, task_prompt, OpencodeFiles,
    OpencodeRecord, OpencodeSpawn, TurnState, WorkerMarkers, OPENCODE_ANSWER_DIR,
    OPENCODE_CLOSE_DIR, OPENCODE_INTERRUPT_DIR, OPENCODE_PROMPT_DIR, OPENCODE_SPAWN_DIR,
};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn entry_ids(store_dir: &Path, dir: &str) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(store_dir.join(dir))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| !name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn queued(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    entries
}

impl OpencodeHost {
    pub(super) fn poll(&self) {
        for id in entry_ids(&self.store_dir, OPENCODE_CLOSE_DIR) {
            self.close(&id);
        }
        for id in entry_ids(&self.store_dir, OPENCODE_ANSWER_DIR) {
            self.answer(&id);
        }
        for id in entry_ids(&self.store_dir, OPENCODE_PROMPT_DIR) {
            self.deliver(&id, false);
        }
        for id in entry_ids(&self.store_dir, OPENCODE_INTERRUPT_DIR) {
            self.deliver(&id, true);
        }
        for id in entry_ids(&self.store_dir, OPENCODE_SPAWN_DIR) {
            self.spawn(&id);
        }
    }

    fn find(&self, id: &str) -> Option<(String, Target)> {
        self.sessions()
            .iter()
            .find(|(_, session)| session.target.worker == id)
            .map(|(session_id, session)| (session_id.clone(), session.target.clone()))
    }

    fn fail(&self, id: &str, message: &str) {
        write_file(&self.files(id).error, message).ok();
    }

    fn spawn(&self, id: &str) {
        let files = self.files(id);
        let request = fs::read_to_string(&files.spawn)
            .ok()
            .and_then(|text| OpencodeSpawn::parse(&text))
            .filter(|request| !request_stale(request.at, now_secs()));
        let Some(request) = request else {
            fs::remove_file(&files.spawn).ok();
            return;
        };
        fs::remove_file(&files.spawn).ok();
        if self.find(id).is_some() {
            self.fail(id, "worker is already running");
            return;
        }
        if let Err(message) = self.create(id, &request) {
            self.fail(id, &message);
        }
    }

    fn create(&self, id: &str, request: &OpencodeSpawn) -> Result<(), String> {
        let api = self.start_server()?;
        let session_id = opencode_session_id(id);
        let directory = request.worktree.to_string_lossy();
        let body = create_session_body(&session_id, &directory, &request.model)?;
        api.post_retrying("/api/session", &body.to_string())?;
        let files = self.files(id);
        let record = OpencodeRecord {
            session_id: session_id.clone(),
            cwd: request.worktree.clone(),
            model: request.model.clone(),
            inbox: request.inbox.clone(),
        };
        write_file(&files.session, &record.render())
            .map_err(|e| format!("could not write the opencode session record: {e}"))?;
        let markers = WorkerMarkers::new(&self.store_dir, id);
        let text = task_prompt(&request.task);
        let mut turn = TurnState::default();
        turn.prompted(&text);
        self.sessions().insert(
            session_id.clone(),
            Session {
                target: Target {
                    worker: id.to_string(),
                    markers: markers.clone(),
                    inbox: request.inbox.clone(),
                },
                turn,
                verified: true,
            },
        );
        if let Err(message) = post_prompt(&api, &session_id, &text) {
            self.sessions().remove(&session_id);
            fs::remove_file(&files.session).ok();
            api.delete(&format!("/api/session/{session_id}")).ok();
            return Err(format!("could not send the first prompt: {message}"));
        }
        write_file(&files.owner, "")
            .and_then(|()| write_file(&markers.ready, ""))
            .map_err(|e| format!("could not write the worker markers: {e}"))
    }

    fn deliver(&self, id: &str, interrupt: bool) {
        let files = self.files(id);
        let dir = if interrupt {
            &files.interrupt
        } else {
            &files.prompt
        };
        for entry in queued(dir) {
            if let (Ok(text), Some((session_id, target))) =
                (fs::read_to_string(&entry), self.find(id))
            {
                if interrupt {
                    self.interrupt(&session_id);
                }
                if let Some(session) = self.sessions().get_mut(&session_id) {
                    session.turn.prompted(&text);
                }
                self.send(&session_id, &target, &text);
            }
            fs::remove_file(&entry).ok();
        }
    }

    fn interrupt(&self, session_id: &str) {
        let result = self
            .api()
            .ok_or_else(|| "the opencode server is not running".to_string())
            .and_then(|api| api.post(&format!("/api/session/{session_id}/interrupt"), ""));
        if let Err(message) = result {
            warn(format!("could not interrupt {session_id}: {message}"));
        }
    }

    fn answer(&self, id: &str) {
        let files = self.files(id);
        let Ok(text) = fs::read_to_string(&files.answer) else {
            return;
        };
        if let Err(message) = self.reply_permission(id, &text) {
            self.fail(id, &message);
        }
        fs::remove_file(&files.answer).ok();
    }

    fn reply_permission(&self, id: &str, text: &str) -> Result<(), String> {
        let (session_id, front) = {
            let sessions = self.sessions();
            let (session_id, session) = sessions
                .iter()
                .find(|(_, session)| session.target.worker == id)
                .ok_or("worker has no running opencode session")?;
            (session_id.clone(), session.turn.blocked.front().cloned())
        };
        let (request_id, decision) = answer_decision(front.as_ref(), text)?;
        let api = self.api().ok_or("the opencode server is not running")?;
        api.post_retrying(
            &permission_reply_path(&session_id, &request_id),
            &permission_reply_body(decision).to_string(),
        )
        .map(|_| ())
    }

    fn close(&self, id: &str) {
        let files = self.files(id);
        let directory = fs::read_to_string(&files.session)
            .ok()
            .and_then(|text| OpencodeRecord::parse(&text))
            .map(|record| record.cwd);
        match self.delete_session(id) {
            Ok(()) => {
                self.evict(directory.as_deref());
                self.forget(id);
            }
            Err(message) => self.fail(id, &message),
        }
        fs::remove_file(&files.close).ok();
    }

    fn evict(&self, directory: Option<&Path>) {
        let (Some(api), Some(directory)) = (self.api(), directory) else {
            return;
        };
        if let Err(message) = api.evict_location(&directory.to_string_lossy()) {
            warn(format!("could not evict the opencode location: {message}"));
        }
    }

    fn delete_session(&self, id: &str) -> Result<(), String> {
        let Some((session_id, _)) = self.find(id) else {
            return Ok(());
        };
        let api = self.api().ok_or("the opencode server is not running")?;
        let running = self
            .sessions()
            .get(&session_id)
            .is_some_and(|session| session.turn.in_flight);
        if running {
            self.interrupt(&session_id);
        }
        match api.delete(&format!("/api/session/{session_id}")) {
            Err(message) if !session_missing(&message) => {
                Err(format!("could not delete the opencode session: {message}"))
            }
            _ => {
                self.sessions().remove(&session_id);
                Ok(())
            }
        }
    }

    fn forget(&self, id: &str) {
        let files = OpencodeFiles::new(&self.store_dir, id);
        fs::remove_file(&files.session).ok();
        fs::remove_file(&files.answer).ok();
        fs::remove_dir_all(&files.prompt).ok();
        fs::remove_dir_all(&files.interrupt).ok();
        fs::remove_file(&WorkerMarkers::new(&self.store_dir, id).ready).ok();
        fs::remove_file(&files.owner).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_ids_list_sorted_visible_entries() {
        let store = tempfile::tempdir().unwrap();
        let dir = store.path().join("d");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("B"), "").unwrap();
        fs::write(dir.join("A"), "").unwrap();
        fs::write(dir.join(".tmp"), "").unwrap();
        assert_eq!(entry_ids(store.path(), "d"), vec!["A", "B"]);
        assert!(entry_ids(store.path(), "missing").is_empty());
    }

    #[test]
    fn queued_skips_hidden_entries_and_sorts() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("2"), "").unwrap();
        fs::write(dir.path().join("1"), "").unwrap();
        fs::write(dir.path().join(".3"), "").unwrap();
        let names: Vec<String> = queued(dir.path())
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["1", "2"]);
    }
}
