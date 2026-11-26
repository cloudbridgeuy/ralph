use ralph_core::worker::{resume_decision, AcpAgent, Resume, SessionRecord, AGY_AGENT};
use std::path::Path;

pub(crate) fn resume(
    agent: &AcpAgent,
    id: &str,
    record: &SessionRecord,
    store: &Path,
    loads: bool,
) -> Resume {
    if *agent != AGY_AGENT {
        return resume_decision(agent, id, record, store, loads);
    }
    let Some(owner) = record.store.as_ref() else {
        return resume_decision(agent, id, record, store, loads);
    };
    match (owner.canonicalize(), store.canonicalize()) {
        (Ok(owner), Ok(store)) => {
            let expected = store.join(ralph_core::worker::INBOX_DIR);
            let inbox = &record.inbox;
            let parent = inbox.parent().and_then(|parent| parent.canonicalize().ok());
            let valid = parent.as_ref() == Some(&store)
                && inbox.file_name() == expected.file_name()
                && match std::fs::symlink_metadata(inbox) {
                    Ok(_) => inbox.canonicalize().ok().as_ref() == Some(&expected),
                    Err(error) => error.kind() == std::io::ErrorKind::NotFound,
                };
            if !valid {
                return Resume::Skip(format!(
                    "agy session {id} not resumed: its inbox belongs to another chat store"
                ));
            }
            if record.cwd.exists()
                && record.cwd.canonicalize().ok().as_deref() != Some(record.cwd.as_path())
            {
                return Resume::Skip(format!(
                    "agy session {id} not resumed: its worktree path changed"
                ));
            }
            let mut record = record.clone();
            record.store = Some(owner);
            resume_decision(agent, id, &record, &store, loads)
        }
        _ => Resume::Skip(format!(
            "agy session {id} not resumed: could not resolve its chat store"
        )),
    }
}

pub(crate) fn lock(owner: &Path) -> std::io::Result<std::fs::File> {
    let store = owner
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| std::io::Error::other("invalid owner path"))?;
    let dir = store.join("agy-lock");
    std::fs::create_dir_all(&dir)?;
    let name = owner
        .file_name()
        .ok_or_else(|| std::io::Error::other("invalid owner name"))?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(name))?;
    file.lock()?;
    Ok(file)
}

pub(super) fn claim(owner: &Path) -> bool {
    let Ok(_lock) = lock(owner) else { return false };
    if owner.exists() && !crate::cursor_owner::reclaim_agy(owner) {
        return false;
    }
    claim_owner(owner).is_ok()
}

pub(super) fn claim_saved(files: &HandoffFiles, record: &SessionRecord) -> bool {
    let Ok(_lock) = lock(&files.owner) else {
        return false;
    };
    let current = fs::read_to_string(&files.session)
        .ok()
        .and_then(|text| SessionRecord::parse(&text));
    if current.as_ref() != Some(record) {
        return false;
    }
    if files.owner.symlink_metadata().is_ok() && !crate::cursor_owner::reclaim_agy(&files.owner) {
        return false;
    }
    claim_owner(&files.owner).is_ok()
}

pub(super) fn owned(owner: &Path) -> bool {
    std::fs::read_to_string(owner)
        .ok()
        .and_then(|text| ralph_core::worker::OwnerRecord::parse(&text))
        .is_some_and(|record| {
            crate::cursor_owner::mine(owner, &record)
                && crate::cursor_owner::process_start(record.chat_pid).as_deref()
                    == Some(&record.chat_start)
        })
}

pub(super) fn remove_owned(owner: &Path) {
    let Ok(_lock) = lock(owner) else { return };
    if owned(owner) {
        std::fs::remove_file(owner).ok();
    }
}

pub(super) fn save(
    opening: &super::Opening,
    session: &agent_client_protocol::schema::v1::SessionId,
    settings: &super::agy_settings::Selection,
) -> Result<(), String> {
    let store = opening
        .inbox
        .parent()
        .ok_or("worker inbox has no store")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let inbox = store.join(
        opening
            .inbox
            .file_name()
            .ok_or("worker inbox has no name")?,
    );
    let record = SessionRecord {
        session_id: session.to_string(),
        cwd: opening.cwd.canonicalize().map_err(|e| e.to_string())?,
        inbox,
        store: Some(store.clone()),
        model: Some(settings.model.clone()),
        effort: settings.effort.clone(),
        permissions: Some(opening.permission_mode),
        scheduled: opening.scheduled,
    };
    crate::cursor_owner::write_file_atomic(
        &ralph_core::worker::HandoffFiles::agy(&store, &opening.worker_id).session,
        &record.render(),
    )
    .map_err(|e| format!("could not save agy session: {e}"))
}

use super::*;

impl CursorHost {
    pub fn cancel_pending(&self, id: &str) {
        let permissions = self.workers().get(id).and_then(CursorWorker::permissions);
        if let Some(permissions) = permissions {
            permissions.cancel_all();
        }
    }

    pub(super) fn close(&self, id: &str) {
        if self.agy && !self.workers().contains_key(id) {
            self.close_saved(id);
            return;
        }
        self.cancel_pending(id);
        let removed = self.workers().remove(id);
        if let Some(mut worker) = removed {
            worker.send(WorkerCommand::Close).ok();
            worker.apply_events();
        }
        self.forget(id, true);
    }

    fn close_saved(&self, id: &str) {
        let files = self.files(id);
        let Ok(_lock) = lock(&files.owner) else {
            return;
        };
        if locked(&self.starting).contains_key(id) {
            return;
        }
        let record = fs::read_to_string(&files.session)
            .ok()
            .and_then(|text| SessionRecord::parse(&text));
        if record.as_ref().is_some_and(|record| {
            resume(&AGY_AGENT, id, record, &self.store_dir, true) == Resume::Go
        }) {
            if files.owner.symlink_metadata().is_ok()
                && !crate::cursor_owner::reclaim_agy(&files.owner)
            {
                return;
            }
            self.cleanup(id, &files, true);
        }
    }

    pub(super) fn cleanup(&self, id: &str, files: &HandoffFiles, close: bool) {
        if self.agy && close && journal::recover(&self.store_dir, id).is_err() {
            return;
        }
        fs::remove_file(&files.answer).ok();
        if !self.agy || close {
            fs::remove_dir_all(&files.prompt).ok();
            fs::remove_dir_all(&files.interrupt).ok();
            if !self.agy {
                fs::remove_dir_all(&files.interrupt_result).ok();
            }
        }
        fs::remove_file(activity_path(&self.store_dir, id)).ok();
        fs::remove_file(gone_path(&self.store_dir, id)).ok();
        if self.agy || close {
            fs::remove_file(ready_path(&self.store_dir, id)).ok();
        }
        if close {
            fs::remove_file(&files.session).ok();
            fs::remove_file(&files.close).ok();
        }
        fs::remove_file(&files.owner).ok();
    }

    pub(super) fn dir(&self, cursor: &'static str, agy: &'static str) -> &'static str {
        if self.agy {
            agy
        } else {
            cursor
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn agy_resume_accepts_store_alias() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = dir.path().join("store");
        let alias = dir.path().join("alias");
        fs::create_dir(&store)?;
        std::os::unix::fs::symlink(&store, &alias)?;
        let record = SessionRecord {
            session_id: "s".into(),
            cwd: store.canonicalize()?,
            inbox: store.join("inbox"),
            store: Some(store),
            model: None,
            effort: None,
            permissions: None,
            scheduled: false,
        };
        assert_eq!(resume(&AGY_AGENT, "A", &record, &alias, true), Resume::Go);
        Ok(())
    }
    #[test]
    fn agy_resume_rejects_inbox_symlink_escape() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = dir.path().join("store");
        fs::create_dir(&store)?;
        let outside = dir.path().join("outside");
        fs::create_dir(&outside)?;
        std::os::unix::fs::symlink(&outside, store.join("inbox"))?;
        let record = SessionRecord {
            session_id: "s".into(),
            cwd: store.clone(),
            inbox: store.join("inbox"),
            store: Some(store.clone()),
            model: None,
            effort: None,
            permissions: None,
            scheduled: false,
        };
        assert!(matches!(
            resume(&AGY_AGENT, "A", &record, &store, true),
            Resume::Skip(_)
        ));
        Ok(())
    }

    #[test]
    fn unreadable_agy_owner_does_not_delete_queued_work() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let files = HandoffFiles::agy(dir.path(), "A");
        write_file(&files.owner, "unreadable")?;
        write_file(&files.prompt.join("1"), "pending")?;
        assert!(!crate::cursor_owner::reclaim_agy(&files.owner));
        assert!(files.owner.exists());
        assert!(files.prompt.join("1").exists());
        Ok(())
    }

    #[test]
    fn reclaim_removes_only_a_dead_owner_and_keeps_pending_work(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let files = HandoffFiles::agy(dir.path(), "A");
        write_file(&files.owner, "dead")?;
        write_file(&files.session, "session")?;
        write_file(&files.prompt.join("1"), "pending")?;
        assert!(crate::cursor_owner::reclaim_agy_decision(
            &files.owner,
            ralph_core::worker::Sweep::Reclaim { kill_group: false }
        ));
        assert!(!files.owner.exists());
        assert!(files.session.exists() && files.prompt.join("1").exists());
        Ok(())
    }

    #[test]
    fn agy_close_clears_an_unowned_local_session() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let files = HandoffFiles::agy(dir.path(), "A");
        let record = SessionRecord {
            session_id: "s".into(),
            cwd: dir.path().canonicalize()?,
            inbox: dir.path().join("inbox"),
            store: Some(dir.path().into()),
            model: None,
            effort: None,
            permissions: None,
            scheduled: false,
        };
        write_file(&files.session, &record.render())?;
        write_file(&files.close, "")?;
        CursorHost::agy(dir.path().into()).poll();
        assert!(!files.session.exists() && !files.close.exists());
        Ok(())
    }

    #[test]
    fn save_writes_confirmed_settings_and_canonical_paths() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let cwd = dir.path().join("work");
        fs::create_dir(&cwd)?;
        let inbox = dir.path().join("inbox");
        let server = serde_json::from_value(acp_mcp_server(
            Path::new("/bin/ralph"),
            "A",
            &inbox,
            &dir.path().join("agy/A"),
            None,
        ))?;
        let opening = Opening {
            agent: AGY_AGENT,
            launch: Launch::Fresh,
            cwd: cwd.clone(),
            server,
            worker_id: "A".into(),
            model: None,
            effort: None,
            permission_mode: PermissionMode::Ask,
            updates: Arc::new(Mutex::new(None)),
            inbox,
            scheduled: false,
        };
        let settings = agy_settings::Selection {
            model: "flash-high".into(),
            effort: Some("high".into()),
        };
        save(&opening, &SessionId::new("s"), &settings)?;
        let text = fs::read_to_string(HandoffFiles::agy(dir.path(), "A").session)?;
        let record = SessionRecord::parse(&text).ok_or("invalid record")?;
        assert_eq!(record.session_id, "s");
        assert_eq!(record.cwd, cwd.canonicalize()?);
        assert_eq!(
            (
                record.model.as_deref(),
                record.effort.as_deref(),
                record.permissions
            ),
            (Some("flash-high"), Some("high"), Some(PermissionMode::Ask))
        );
        Ok(())
    }

    #[test]
    fn agy_resume_rejects_a_replaced_worktree_symlink() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let base = dir.path().canonicalize()?;
        let cwd = base.join("work");
        let other = base.join("other");
        fs::create_dir(&other)?;
        std::os::unix::fs::symlink(&other, &cwd)?;
        let record = SessionRecord {
            session_id: "s".into(),
            cwd,
            inbox: dir.path().join("inbox"),
            store: Some(dir.path().into()),
            model: None,
            effort: None,
            permissions: None,
            scheduled: false,
        };
        assert!(matches!(
            resume(&AGY_AGENT, "A", &record, dir.path(), true),
            Resume::Skip(_)
        ));
        Ok(())
    }

    #[test]
    fn agy_restart_reports_a_persisted_unfinished_turn() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let files = HandoffFiles::agy(dir.path(), "A");
        let record = SessionRecord {
            session_id: "s".into(),
            cwd: dir.path().join("missing"),
            inbox: dir.path().join("inbox"),
            store: Some(dir.path().into()),
            model: None,
            effort: None,
            permissions: None,
            scheduled: false,
        };
        write_file(&files.session, &record.render())?;
        let report = ralph_core::worker::prompt_failure_report(
            "agy",
            &ralph_core::worker::WorkerId::parse("A").unwrap(),
            "2026-01-01T00:00:00Z",
            "unfinished",
        );
        write_file(
            &dir.path()
                .join("agy-pending/A/20260101000000-000000-A.json"),
            &serde_json::to_string(&report)?,
        )?;
        HeadlessHost::resume(CursorHost::agy(dir.path().into()).as_ref());
        assert!(dir.path().join("inbox").is_dir());
        assert_eq!(fs::read_dir(dir.path().join("inbox"))?.count(), 1);
        Ok(())
    }

    #[test]
    fn changed_saved_session_is_not_claimed() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let files = HandoffFiles::agy(dir.path(), "A");
        let old = SessionRecord {
            session_id: "old".into(),
            cwd: dir.path().canonicalize()?,
            inbox: dir.path().join("inbox"),
            store: Some(dir.path().into()),
            model: None,
            effort: None,
            permissions: None,
            scheduled: false,
        };
        let mut current = old.clone();
        current.session_id = "new".into();
        write_file(&files.session, &current.render())?;
        assert!(!claim_saved(&files, &old));
        assert!(!files.owner.exists());
        Ok(())
    }

    #[test]
    fn actual_default_model_is_recorded() {
        let state = serde_json::json!({"models": {"currentModelId": "flash-high"}, "configOptions": [{"id": "thought_level", "currentValue": "high"}]});
        let settings = super::super::agy_settings::selection(&state, &state, None);
        assert_eq!(
            settings.map(|settings| (settings.model, settings.effort)),
            Ok(("flash-high".into(), Some("high".into())))
        );
    }

    #[test]
    fn agy_resume_enumerates_its_sessions() -> Result<(), Box<dyn std::error::Error>> {
        use super::super::*;
        let dir = tempfile::tempdir()?;
        let files = HandoffFiles::agy(dir.path(), "A");
        let record = SessionRecord {
            session_id: "s".into(),
            cwd: dir.path().join("missing"),
            inbox: dir.path().join("inbox"),
            store: Some(dir.path().into()),
            model: None,
            effort: None,
            permissions: None,
            scheduled: false,
        };
        write_file(&files.session, &record.render())?;
        let host = CursorHost::agy(dir.path().into());
        HeadlessHost::resume(host.as_ref());
        assert!(files.error.exists());
        Ok(())
    }
}

pub(super) fn exit_reason(status: ExitStatus, agent: &AcpAgent) -> String {
    format!("{} acp exited ({status})", agent.command)
}

pub(super) fn agent_version_warning(agent: &AcpAgent, inbox: &Path) -> Option<String> {
    let probe = agent_version_probe(agent);
    crate::version_drift::soft_warning(
        &CURSOR_GATE,
        probed_version(agent, &probe).map(str::to_string),
        inbox,
    )
}

#[cfg(test)]
mod exit_tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn exit_reason_uses_the_launched_agent_command() {
        assert_eq!(
            exit_reason(
                ExitStatus::from_raw(9),
                &AcpAgent {
                    command: "agy",
                    ..CURSOR_AGENT
                }
            ),
            "agy acp exited (signal: 9 (SIGKILL))"
        );
    }

    #[test]
    fn exit_reason_names_the_agent_and_the_status() {
        assert_eq!(
            exit_reason(ExitStatus::from_raw(9), &CURSOR_AGENT),
            "agent acp exited (signal: 9 (SIGKILL))"
        );
    }
}
