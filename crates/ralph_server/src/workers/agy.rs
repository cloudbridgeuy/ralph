use super::*;

pub(super) fn saved_agy(store_dir: &Path, files: &HandoffFiles) -> bool {
    let Some(record) = fs::read_to_string(&files.session)
        .ok()
        .and_then(|text| ralph_core::worker::SessionRecord::parse(&text))
    else {
        return false;
    };
    let id = files
        .session
        .file_name()
        .and_then(|id| id.to_str())
        .unwrap_or_default();
    crate::acp_worker::recovery::resume(
        &ralph_core::worker::AGY_AGENT,
        id,
        &record,
        store_dir,
        true,
    ) == ralph_core::worker::Resume::Go
}

impl Workers {
    pub(super) fn check_agy(&self, id: &str, closing: bool) -> io::Result<()> {
        let files = HandoffFiles::agy(&self.store_dir, id);
        let present = |path: &Path| {
            path.symlink_metadata()
                .map_or_else(|error| error.kind() != io::ErrorKind::NotFound, |_| true)
        };
        if !files.all().iter().any(|path| present(path))
            && !present(&files.session)
            && !present(&self.store_dir.join("agy-pending").join(id))
        {
            return Ok(());
        }
        let _lock = crate::acp_worker::recovery::lock(&files.owner)?;
        self.check_agy_locked(&files, id, closing)
    }

    pub(super) fn check_agy_locked(
        &self,
        files: &HandoffFiles,
        id: &str,
        closing: bool,
    ) -> io::Result<()> {
        let present = |path: &Path| {
            path.symlink_metadata()
                .map_or_else(|error| error.kind() != io::ErrorKind::NotFound, |_| true)
        };
        if present(&files.owner) {
            let owner = fs::read_to_string(&files.owner)
                .ok()
                .and_then(|text| ralph_core::worker::OwnerRecord::parse(&text));
            let identity = owner
                .as_ref()
                .map(|owner| crate::cursor_owner::agy_identity(owner.chat_pid, &owner.chat_start));
            if !closing
                || match identity {
                    Some(crate::cursor_owner::ProcessIdentity::Matches) => !caller_owns(files),
                    Some(crate::cursor_owner::ProcessIdentity::GoneOrReused) => {
                        !saved_agy(&self.store_dir, files)
                    }
                    _ => true,
                }
            {
                return Err(io::Error::other(format!(
                    "worker {id} is owned by another or unreadable agy chat"
                )));
            }
        }
        if present(&files.session) && (!closing || !saved_agy(&self.store_dir, files)) {
            return Err(io::Error::other(format!(
                "worker {id} has a saved or unreadable agy session"
            )));
        }
        let has_entries = |dir: &Path| match fs::read_dir(dir) {
            Ok(mut entries) => entries.next().is_some(),
            Err(error) => error.kind() != io::ErrorKind::NotFound,
        };
        if !closing
            && (present(&files.spawn)
                || has_entries(&files.prompt)
                || has_entries(&files.interrupt)
                || has_entries(&self.store_dir.join("agy-pending").join(id)))
        {
            return Err(io::Error::other(format!(
                "worker {id} has pending agy work"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_preflight_preserves_unreadable_owner_and_queued_work(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let files = HandoffFiles::agy(dir.path(), "A");
        write_file(&files.owner, "unreadable")?;
        write_file(&files.prompt.join("1"), "pending")?;
        write_file(&files.interrupt_result.join("1"), "gone")?;
        assert!(Workers::new(
            dir.path().into(),
            PathBuf::from("ralph"),
            String::new(),
            None
        )
        .refuse_busy_headless("A", false)
        .is_err());
        assert!(
            files.owner.exists()
                && files.prompt.join("1").exists()
                && files.interrupt_result.join("1").exists()
        );
        Ok(())
    }

    #[test]
    fn unrelated_preflight_does_not_create_agy_lock() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        Workers::new(
            dir.path().into(),
            PathBuf::from("ralph"),
            String::new(),
            None,
        )
        .refuse_busy_headless("A", false)?;
        assert!(!dir.path().join("agy-lock").exists());
        Ok(())
    }

    #[test]
    fn saved_local_agy_session_is_discovered_for_close() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let files = HandoffFiles::agy(dir.path(), "A");
        let record = ralph_core::worker::SessionRecord {
            session_id: "s".into(),
            cwd: dir.path().canonicalize()?,
            inbox: dir.path().join("inbox"),
            store: Some(dir.path().into()),
            model: None,
            effort: None,
            permissions: None,
        };
        write_file(&files.session, &record.render())?;
        let workers = Workers::new(
            dir.path().into(),
            PathBuf::from("ralph"),
            String::new(),
            None,
        );
        assert_eq!(
            workers.headless_files("A").map(|(kind, _)| kind),
            Some(HarnessId::Agy)
        );
        assert!(workers.refuse_busy_headless("A", true).is_ok());
        Ok(())
    }

    #[test]
    fn agy_preflight_refuses_an_existing_saved_session() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let files = HandoffFiles::agy(dir.path(), "A");
        write_file(&files.session, "saved")?;
        assert!(Workers::new(
            dir.path().into(),
            PathBuf::from("ralph"),
            String::new(),
            None
        )
        .refuse_busy_headless("A", false)
        .is_err());
        assert!(files.session.exists());
        Ok(())
    }
}
