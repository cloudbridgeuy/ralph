use ralph_core::worker::{resume_decision, AcpAgent, Resume, SessionRecord, AGY_AGENT};
use std::path::Path;

pub(super) fn resume(
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
            if record
                .inbox
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .as_ref()
                != Some(&store)
            {
                return Resume::Skip(format!(
                    "agy session {id} not resumed: its inbox belongs to another chat store"
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
    };
    crate::cursor_owner::write_file_atomic(
        &ralph_core::worker::HandoffFiles::agy(&store, &opening.worker_id).session,
        &record.render(),
    )
    .map_err(|e| format!("could not save agy session: {e}"))
}

use super::*;

impl CursorHost {
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
            cwd: store.clone(),
            inbox: store.join("inbox"),
            store: Some(store),
            model: None,
            effort: None,
            permissions: None,
        };
        assert_eq!(resume(&AGY_AGENT, "A", &record, &alias, true), Resume::Go);
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
        };
        write_file(&files.session, &record.render())?;
        let host = CursorHost::agy(dir.path().into());
        host.resume_all();
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
