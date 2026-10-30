use super::{post_prompt, record_usage, OpencodeHost, Session, Target};
use crate::acp_worker::Events;
use crate::cursor_owner::{now_secs, write_file};
use crate::warn::warn;
use crate::workers::headless::Heartbeat;
use ralph_core::snapshot::WorkerSite;
use ralph_core::worker::{
    create_session_body, interrupt_result_text, opencode_session_id, request_stale, task_prompt,
    HandoffFiles, HeadlessState, HeadlessWorker, OpencodeRecord, OpencodeSpawn, OpencodeTurnUsage,
    PermissionAnswer, TurnState, WorkerCommand, WorkerEvent, WorkerMarkers, OPENCODE_ANSWER_DIR,
    OPENCODE_CLOSE_DIR, OPENCODE_INTERRUPT_DIR, OPENCODE_PREFLIGHT_DIR, OPENCODE_PROMPT_DIR,
    OPENCODE_SPAWN_DIR,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
        for id in entry_ids(&self.store_dir, OPENCODE_PREFLIGHT_DIR) {
            self.preflight(&id);
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

    fn preflight(&self, id: &str) {
        let preflight = HandoffFiles::opencode_preflight(&self.store_dir, id);
        let fresh = fs::read_to_string(&preflight)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .is_some_and(|at| !request_stale(at, now_secs()));
        fs::remove_file(&preflight).ok();
        if !fresh {
            return;
        }
        let healthy_path = HandoffFiles::opencode_healthy(&self.store_dir, id);
        let healthy = self.start_server().and_then(|_| {
            write_file(&healthy_path, "")
                .map_err(|e| format!("could not write the opencode healthy marker: {e}"))
        });
        if let Err(message) = healthy {
            self.fail(id, &message);
        }
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
        let heartbeat = Heartbeat::new(markers.activity.clone(), markers.gone.clone());
        heartbeat.clear();
        heartbeat.set_running(true);
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
                    site: request.snapshots.then(|| WorkerSite {
                        store: self.store_dir.clone(),
                        id: id.to_string(),
                        worktree: request.worktree.clone(),
                    }),
                },
                turn,
                verified: true,
                heartbeat,
                state: HeadlessState::default(),
                events: Events::default(),
                usage: OpencodeTurnUsage::new(&request.model),
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
                    self.request_interrupt(&session_id, true);
                    let outcome = self.interrupt(id, &session_id, &text);
                    if outcome != Some(true) {
                        self.request_interrupt(&session_id, false);
                    }
                    self.write_interrupt_result(id, &entry, outcome);
                }
                if let Some(session) = self.sessions().get_mut(&session_id) {
                    session.turn.prompted(&text);
                    session.heartbeat.set_running(true);
                }
                self.send(&session_id, &target, &text);
            }
            fs::remove_file(&entry).ok();
        }
    }

    fn request_interrupt(&self, session_id: &str, requested: bool) {
        if let Some(session) = self.sessions().get_mut(session_id) {
            session.turn.interrupt_requested = requested;
        }
    }

    fn interrupt(&self, id: &str, session_id: &str, text: &str) -> Option<bool> {
        self.worker(id, session_id)
            .send(WorkerCommand::Interrupt(text.to_string()))
            .ok();
        self.apply_events(id, session_id)
            .into_iter()
            .find_map(|event| match event {
                WorkerEvent::Interrupted { outcome, .. } => outcome,
                _ => None,
            })
    }

    fn write_interrupt_result(&self, id: &str, entry: &Path, outcome: Option<bool>) {
        let (Some(cancelled), Some(name)) = (outcome, entry.file_name()) else {
            return;
        };
        let results = self.files(id).interrupt_result;
        let pending = results.join(format!(".{}", name.to_string_lossy()));
        if write_file(&pending, interrupt_result_text(cancelled)).is_ok() {
            fs::rename(&pending, results.join(name)).ok();
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
        let answer = PermissionAnswer::decode(text)?;
        let (session_id, heartbeat) = {
            let sessions = self.sessions();
            let (session_id, session) = sessions
                .iter()
                .find(|(_, session)| session.target.worker == id)
                .ok_or("worker has no running opencode session")?;
            (session_id.clone(), Arc::clone(&session.heartbeat))
        };
        self.worker(id, &session_id)
            .send(WorkerCommand::AnswerPermission(answer))
            .map(|()| heartbeat.touch())
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
        let Some(session_id) = self
            .find(id)
            .map(|(session_id, _)| session_id)
            .or_else(|| self.recorded_session_id(id))
        else {
            return Ok(());
        };
        let running = self
            .sessions()
            .get(&session_id)
            .is_some_and(|session| session.turn.in_flight);
        if running {
            self.interrupt(id, &session_id, "");
        }
        self.worker(id, &session_id)
            .send(WorkerCommand::Close)
            .inspect(|()| {
                self.apply_events(id, &session_id);
                let removed = self.sessions().remove(&session_id);
                if let Some(mut session) = removed {
                    record_usage(&session_id, &session.target, None, &session.usage.take());
                }
            })
    }

    fn recorded_session_id(&self, id: &str) -> Option<String> {
        fs::read_to_string(&self.files(id).session)
            .ok()
            .and_then(|text| OpencodeRecord::parse(&text))
            .map(|record| record.session_id)
    }

    fn forget(&self, id: &str) {
        let files = HandoffFiles::opencode(&self.store_dir, id);
        fs::remove_file(&files.session).ok();
        fs::remove_file(&files.answer).ok();
        fs::remove_dir_all(&files.prompt).ok();
        fs::remove_dir_all(&files.interrupt).ok();
        fs::remove_dir_all(&files.interrupt_result).ok();
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
