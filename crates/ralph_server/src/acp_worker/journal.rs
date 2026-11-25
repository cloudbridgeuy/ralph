use super::*;
use crate::schedule_store::scheduled_tasks;
use ralph_core::schedule_resume::has_open_run;
use ralph_core::worker::{inbox_file_name, prompt_failure_report, WorkerId};
use std::sync::atomic::AtomicU64;

static SEQ: AtomicU64 = AtomicU64::new(0);

fn turn_path(store: &Path, id: &str) -> PathBuf {
    store.join("agy-turn").join(id)
}

pub(crate) fn encode_interrupt(store: &Path, id: &str, text: &str) -> String {
    let turn = fs::read_to_string(turn_path(store, id)).ok();
    serde_json::json!({"agy_interrupt": 1, "text": text, "turn": turn}).to_string()
}

fn decode_interrupt(text: &str, entry: &str) -> (String, Option<String>) {
    if !entry.ends_with("-agy1") {
        return (text.to_string(), None);
    }
    let parsed = serde_json::from_str::<serde_json::Value>(text).ok();
    match parsed
        .as_ref()
        .filter(|value| value.get("agy_interrupt").and_then(|v| v.as_u64()) == Some(1))
        .and_then(|value| value.get("text"))
        .and_then(|value| value.as_str())
    {
        Some(message) => (
            message.to_string(),
            parsed
                .as_ref()
                .and_then(|value| value.get("turn"))
                .and_then(|value| value.as_str())
                .map(str::to_string),
        ),
        None => (text.to_string(), None),
    }
}

fn token(store: &Path, id: &str, generation: u64, group: u32) -> Option<String> {
    let owner = fs::read_to_string(HandoffFiles::agy(store, id).owner).ok()?;
    let record = ralph_core::worker::OwnerRecord::parse(&owner)?;
    (record.group == group).then(|| ralph_core::worker::turn_token(&record, generation))
}

pub(super) fn publish_turn(
    store: &Path,
    id: &str,
    generation: u64,
    group: u32,
) -> std::io::Result<String> {
    let owner = HandoffFiles::agy(store, id).owner;
    let _lock = super::recovery::lock(&owner)?;
    let token = token(store, id, generation, group)
        .ok_or_else(|| std::io::Error::other("agy owner changed"))?;
    crate::cursor_owner::write_file_atomic(&turn_path(store, id), &token)?;
    Ok(token)
}

pub(super) fn finish_turn(store: &Path, id: &str, token: &str) {
    let owner = HandoffFiles::agy(store, id).owner;
    let Ok(_lock) = super::recovery::lock(&owner) else {
        return;
    };
    let path = turn_path(store, id);
    if fs::read_to_string(&path).ok().as_deref() == Some(token) {
        fs::remove_file(path).ok();
    }
}

fn interrupt_target(
    text: &str,
    entry: &str,
    current: Option<String>,
    generation: u64,
) -> (String, u64) {
    let (message, target) = decode_interrupt(text, entry);
    let target = match target {
        Some(target) if current.as_deref() == Some(&target) => generation,
        Some(_) => u64::MAX,
        None => 0,
    };
    (message, target)
}

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
    let tasks = scheduled_tasks(store);
    if WorkerId::parse(id).is_ok_and(|worker| has_open_run(&tasks, &worker)) {
        return fs::remove_file(report);
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
            let store = dir
                .parent()
                .and_then(Path::parent)
                .ok_or("invalid agy journal")?;
            let id = dir
                .file_name()
                .and_then(|id| id.to_str())
                .ok_or("invalid worker id")?;
            let (message, target) = interrupt_target(
                text,
                entry,
                token(store, id, generation, self.child.id()),
                generation,
            );
            let receipt = journal::record(dir)?;
            Inbound::TrackedInterrupt(message, entry.to_string(), receipt, target)
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
    fn sender_token_binds_interrupt_and_legacy_json_stays_raw(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let store = tempfile::tempdir()?;
        let owner = HandoffFiles::agy(store.path(), "A").owner;
        let record = ralph_core::worker::OwnerRecord {
            chat_pid: 1,
            chat_start: "chat".into(),
            group: 7,
            group_start: "agent".into(),
            instance: None,
        };
        write_file(&owner, &record.render())?;
        let old = publish_turn(store.path(), "A", 1, 7)?;
        let payload = encode_interrupt(store.path(), "A", "stop");
        assert_eq!(
            interrupt_target(&payload, "1-agy1", Some(old.clone()), 1),
            ("stop".into(), 1)
        );
        let new = publish_turn(store.path(), "A", 3, 7)?;
        assert_eq!(
            interrupt_target(&payload, "1-agy1", Some(new.clone()), 3),
            ("stop".into(), u64::MAX)
        );
        finish_turn(store.path(), "A", &old);
        assert_eq!(fs::read_to_string(turn_path(store.path(), "A"))?, new);
        let raw = r#"{"agy_interrupt":1,"text":"do not rewrite","turn":"fake"}"#;
        assert_eq!(
            interrupt_target(raw, "1-123", Some(new), 3),
            (raw.into(), 0)
        );
        finish_turn(
            store.path(),
            "A",
            &fs::read_to_string(turn_path(store.path(), "A"))?,
        );
        assert!(!turn_path(store.path(), "A").exists());
        Ok(())
    }

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
