use super::tests::workers_in;
use super::*;

#[test]
fn agy_interrupt_queue_preserves_sender_turn_token() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let workers = workers_in(dir.path());
    let files = HandoffFiles::agy(dir.path(), "A");
    write_file(&dir.path().join("agy-turn/A"), "owner:1")?;
    let payload = crate::acp_worker::journal::encode_interrupt(dir.path(), "A", "stop");
    let entry = workers.queue_entry_with_suffix(&files.interrupt, &payload, "-agy1")?;
    assert!(entry.ends_with("-agy1"));
    write_file(&dir.path().join("agy-turn/A"), "owner:3")?;
    let queued = std::fs::read_to_string(files.interrupt.join(entry))?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&queued).ok(),
        Some(serde_json::json!({"agy_interrupt":1, "text":"stop", "turn":"owner:1"}))
    );
    Ok(())
}
