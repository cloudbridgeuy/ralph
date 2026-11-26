use super::*;

fn rejected_files(inbox_dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(inbox_dir.join(REJECTED_DIR))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    paths
}

fn sync_reports(inbox: &Inbox, warnings: &mut Vec<String>) -> Vec<Report> {
    inbox
        .sync_with(&mut |w| warnings.push(w), |l| {
            (l.reports(), LedgerChange::Unchanged)
        })
        .unwrap()
}

#[test]
fn malformed_message_file_is_skipped_and_others_delivered() {
    let dir = tempfile::tempdir().unwrap();
    let inbox_dir = dir.path().join("inbox");
    let inbox = Inbox::new(inbox_dir.clone());
    let post = |text: &str, seq| {
        let report = Report::from_worker(
            &WorkerId::parse("A").unwrap(),
            format!("2026-01-01T00:00:0{seq}Z").parse().unwrap(),
            ReportStatus::Done,
            text.to_string(),
        );
        inbox.post(&report, seq).unwrap();
    };
    post("first", 1);
    fs::write(
        inbox_dir.join("2026-01-01T00-00-01.5Z-bad.json"),
        "not json",
    )
    .unwrap();
    post("second", 2);

    let delivered = inbox.drain(Selection::All).unwrap();
    let texts: Vec<&str> = delivered.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, ["first", "second"]);
    assert!(!inbox_dir.join("2026-01-01T00-00-01.5Z-bad.json").exists());
    let rejected = inbox_dir
        .join("rejected")
        .join("2026-01-01T00-00-01.5Z-bad.json");
    assert_eq!(fs::read_to_string(&rejected).unwrap(), "not json");
}

#[test]
fn rejected_message_file_is_not_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let inbox_dir = dir.path().join("inbox");
    fs::create_dir_all(&inbox_dir).unwrap();
    let bad = inbox_dir.join("bad.json");
    fs::write(&bad, "{broken").unwrap();
    let inbox = Inbox::new(inbox_dir.clone());

    let mut warnings = Vec::new();
    assert!(sync_reports(&inbox, &mut warnings).is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(!bad.exists());
    let rejected = inbox_dir.join("rejected").join("bad.json");
    assert!(rejected.exists());
    assert!(!is_message(&rejected));
    assert!(inbox.pending().unwrap().is_empty());

    assert!(sync_reports(&inbox, &mut warnings).is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(fs::read_to_string(&rejected).unwrap(), "{broken");
}

#[test]
fn load_ledger_keeps_good_entries_when_one_is_malformed() {
    let dir = tempfile::tempdir().unwrap();
    let inbox_dir = dir.path().join("inbox");
    let inbox = Inbox::new(inbox_dir.clone());
    let raw = serde_json::json!({
        "entries": [
            {"source": "a", "report": {"worker": "A", "at": "2026-01-01T00:00:01Z", "status": "done", "text": "ok"}},
            {"source": "bad", "report": {"worker": "A"}},
            {"source": "b", "report": {"worker": "B", "at": "2026-01-01T00:00:02Z", "status": "done", "text": "ok2"}},
        ],
    });
    let original = serde_json::to_vec(&raw).unwrap();
    let ledger_path = dir.path().join("inbox.json");
    fs::write(&ledger_path, &original).unwrap();

    let mut warnings = Vec::new();
    let reports = sync_reports(&inbox, &mut warnings);
    let workers: Vec<String> = reports.iter().map(|r| r.from.to_string()).collect();
    assert_eq!(workers, ["A", "B"]);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("bad"));

    let copies = rejected_files(&inbox_dir);
    assert_eq!(copies.len(), 1);
    assert!(!is_message(&copies[0]));
    assert_eq!(fs::read(&copies[0]).unwrap(), original);
    let (repaired, skipped) = Ledger::decode(&fs::read(&ledger_path).unwrap());
    assert!(skipped.is_empty());
    assert_eq!(repaired.reports().len(), 2);

    let second = sync_reports(&inbox, &mut warnings);
    assert_eq!(second.len(), 2);
    assert_eq!(warnings.len(), 1);
    assert_eq!(rejected_files(&inbox_dir).len(), 1);
}

#[test]
fn load_ledger_returns_empty_when_file_is_not_json() {
    let dir = tempfile::tempdir().unwrap();
    let inbox_dir = dir.path().join("inbox");
    let inbox = Inbox::new(inbox_dir.clone());
    fs::write(dir.path().join("inbox.json"), "not json at all").unwrap();

    let mut warnings = Vec::new();
    assert!(sync_reports(&inbox, &mut warnings).is_empty());
    assert_eq!(warnings.len(), 1);
    let copies = rejected_files(&inbox_dir);
    assert_eq!(copies.len(), 1);
    assert_eq!(fs::read_to_string(&copies[0]).unwrap(), "not json at all");

    assert!(sync_reports(&inbox, &mut warnings).is_empty());
    assert_eq!(warnings.len(), 1);
    assert_eq!(rejected_files(&inbox_dir).len(), 1);
}

#[test]
fn read_only_load_warns_without_copying_or_rewriting_corrupt_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let inbox_dir = dir.path().join("inbox");
    let inbox = Inbox::new(inbox_dir.clone());
    let ledger_path = dir.path().join("inbox.json");
    fs::write(&ledger_path, "not json at all").unwrap();

    let mut warnings = Vec::new();
    let loaded = inbox
        .load_ledger_with(LoadMode::ReadOnly, &mut |w| warnings.push(w))
        .unwrap();
    assert!(loaded.ledger.entries().is_empty());
    assert_eq!(loaded.state, LedgerState::Unchanged);
    assert_eq!(warnings.len(), 1);
    assert!(!inbox_dir.join(REJECTED_DIR).exists());
    assert_eq!(fs::read_to_string(&ledger_path).unwrap(), "not json at all");
}

fn post_past_taken_names(taken: u64) -> (io::Result<()>, usize) {
    let dir = tempfile::tempdir().unwrap();
    let inbox_dir = dir.path().join("inbox");
    fs::create_dir_all(&inbox_dir).unwrap();
    let report = Report::from_worker(
        &WorkerId::parse("A").unwrap(),
        "2026-01-01T00:00:00Z".parse().unwrap(),
        ReportStatus::Done,
        "late".to_string(),
    );
    for seq in 0..taken {
        fs::write(inbox_dir.join(inbox_file_name(&report, seq)), "").unwrap();
    }
    let result = Inbox::new(inbox_dir.clone()).post(&report, 0);
    (result, fs::read_dir(&inbox_dir).unwrap().count())
}

#[test]
fn post_takes_the_last_of_a_thousand_names() {
    let (result, files) = post_past_taken_names(999);
    assert!(result.is_ok());
    assert_eq!(files, 1000);
}

#[test]
fn post_gives_up_after_a_thousand_taken_names() {
    let (result, files) = post_past_taken_names(1000);
    assert_eq!(
        result.map_err(|e| e.kind()),
        Err(io::ErrorKind::AlreadyExists)
    );
    assert_eq!(files, 1000);
}
