use super::*;
use ralph_core::worker::{inbox_file_name, prompt_failure_report};
use std::sync::atomic::AtomicU64;

static SEQ: AtomicU64 = AtomicU64::new(0);

pub(super) fn directory(store: &Path, id: &str) -> PathBuf {
    store.join("agy-pending").join(id)
}

pub(super) fn record(dir: &Path) -> Result<String, String> {
    let id = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid worker id")?;
    let at = chrono::Utc::now().to_rfc3339();
    let report = prompt_failure_report(
        "agy",
        id,
        &at,
        "the chat ended before accepted work finished; its outcome is unknown. Review the work and resend any message that is still needed",
    );
    let name = inbox_file_name(&report, SEQ.fetch_add(1, Ordering::Relaxed));
    let text = serde_json::to_string(&report).map_err(|e| e.to_string())?;
    crate::cursor_owner::write_file_atomic(&dir.join(&name), &text)
        .map_err(|e| format!("could not persist agy work: {e}"))?;
    Ok(name)
}

pub(super) fn finish(dir: &Path, receipt: Option<&str>) {
    if let Some(receipt) = receipt {
        fs::remove_file(dir.join(receipt)).ok();
    }
}

pub(super) fn recover(store: &Path, id: &str) -> std::io::Result<()> {
    let dir = directory(store, id);
    let mut receipts = crate::cursor_owner::queued_entries(&dir);
    let files = HandoffFiles::agy(store, id);
    let pending = crate::cursor_owner::queued_entries(&files.prompt);
    let interrupts = crate::cursor_owner::queued_entries(&files.interrupt);
    if receipts.is_empty() && (!pending.is_empty() || !interrupts.is_empty()) {
        let receipt = record(&dir).map_err(std::io::Error::other)?;
        receipts.push(dir.join(receipt));
    }
    let Some(report) = receipts.pop() else {
        return Ok(());
    };
    for entry in pending {
        let name = entry
            .file_name()
            .ok_or_else(|| std::io::Error::other("invalid prompt entry"))?
            .to_string_lossy();
        fs::rename(
            &entry,
            entry.with_file_name(ralph_core::worker::lost_marker_name(&name)),
        )?;
    }
    for entry in interrupts {
        let name = entry
            .file_name()
            .ok_or_else(|| std::io::Error::other("invalid interrupt entry"))?;
        crate::cursor_owner::write_file_atomic(&files.interrupt_result.join(name), "gone")?;
        fs::remove_file(entry)?;
    }
    for duplicate in receipts {
        fs::remove_file(duplicate)?;
    }
    let inbox = store.join(ralph_core::worker::INBOX_DIR);
    fs::create_dir_all(&inbox)?;
    let name = report
        .file_name()
        .ok_or_else(|| std::io::Error::other("invalid agy receipt"))?;
    fs::rename(&report, inbox.join(name))
}

impl AcpWorker {
    pub(super) fn apply_events(&mut self) {
        for event in self.poll() {
            let (state, effects) = reduce(&self.state, &event);
            self.state = state;
            for effect in &effects {
                apply_effect(&self.markers, effect);
            }
        }
    }

    pub(super) fn interrupt(&self, text: &str, entry: &str) -> Result<(), String> {
        let item = if let Some(dir) = &self.journal {
            let generation = self.generation.load(Ordering::SeqCst);
            let receipt = journal::record(dir)?;
            Inbound::TrackedInterrupt(text.to_string(), entry.to_string(), receipt, generation)
        } else {
            Inbound::Interrupt(text.to_string(), entry.to_string())
        };
        self.prompts.unbounded_send(item).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_agy_host_leaves_another_hosts_queued_work_alone(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let store = tempfile::tempdir()?;
        let files = HandoffFiles::agy(store.path(), "A");
        write_file(&files.prompt.join("1"), "queued")?;
        let host = CursorHost::agy(store.path().into());
        host.poll();
        assert!(files.prompt.join("1").exists());
        Ok(())
    }

    #[test]
    fn completed_work_does_not_make_a_recovery_report() -> Result<(), Box<dyn std::error::Error>> {
        let store = tempfile::tempdir()?;
        let dir = directory(store.path(), "A");
        let receipt = record(&dir)?;
        finish(&dir, Some(&receipt));
        recover(store.path(), "A")?;
        assert!(!store.path().join("inbox").exists());
        Ok(())
    }

    #[test]
    fn recovery_transfers_one_report_and_never_replays_queued_work(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let store = tempfile::tempdir()?;
        let dir = directory(store.path(), "A");
        record(&dir)?;
        record(&dir)?;
        let files = HandoffFiles::agy(store.path(), "A");
        write_file(&files.prompt.join("1"), "queued")?;
        recover(store.path(), "A")?;
        assert!(!files.prompt.join("1").exists());
        assert_eq!(fs::read_dir(store.path().join("inbox"))?.count(), 1);
        recover(store.path(), "A")?;
        assert_eq!(fs::read_dir(store.path().join("inbox"))?.count(), 1);
        Ok(())
    }
}
