use super::*;
use ralph_core::chat_memory::{decode_rows, Role};
use std::cell::RefCell;

fn store_with(entries: usize) -> (tempfile::TempDir, TagStore) {
    let dir = tempfile::tempdir().unwrap();
    let log: String = (0..entries)
        .map(|i| {
            format!(
                "{{\"role\":\"user\",\"at\":\"2026-11-09T00:00:0{}Z\",\"text\":\"m{i}\"}}\n",
                i % 10
            )
        })
        .collect();
    fs::write(dir.path().join("log.jsonl"), log).unwrap();
    let store = TagStore::new(dir.path().to_path_buf());
    (dir, store)
}

fn grow(dir: &tempfile::TempDir, to: usize, from: usize) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(dir.path().join("log.jsonl"))
        .unwrap();
    for i in from..to {
        writeln!(
            file,
            "{{\"role\":\"assistant\",\"at\":\"2026-11-09T00:00:00Z\",\"text\":\"m{i}\"}}"
        )
        .unwrap();
    }
}

fn state(dir: &tempfile::TempDir) -> TagState {
    decode_rows(&fs::read(dir.path().join(TAGS_FILE)).unwrap())
}

fn asked_ids(batch: &str) -> Vec<usize> {
    batch
        .lines()
        .filter_map(|l| l.strip_prefix("### ENTRY "))
        .filter_map(|l| l.split(' ').next()?.parse().ok())
        .collect()
}

fn reply_for(ids: &[usize], rule: impl Fn(usize) -> bool) -> String {
    let rows: Vec<String> = ids
        .iter()
        .map(|n| {
            format!(
                "{{\"n\":{n},\"rule\":{},\"decision\":false,\"approval\":false}}",
                rule(*n)
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
}

fn now() -> String {
    "2026-11-09T00:00:00Z".to_string()
}

/// A subprocess forked by a parallel test can hold a copy of a just-released
/// `TAG` lock until it execs, so a pass may report `Busy` for a moment.
fn pass(store: &TagStore, ask: &mut Ask) -> io::Result<Report> {
    for _ in 0..500 {
        if let Pass::Done(report) = store.run_pass(&now, ask)? {
            return Ok(report);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err(io::Error::other("TAG stayed busy"))
}

#[test]
fn the_first_pass_only_writes_the_start_row_and_asks_nothing() {
    let (dir, store) = store_with(4);
    let mut ask = |_: &str| -> Result<String, String> { panic!("no call expected") };
    pass(&store, &mut ask).unwrap();
    let text = fs::read_to_string(dir.path().join(TAGS_FILE)).unwrap();
    assert_eq!(
        text,
        "{\"v\":1,\"start\":4,\"at\":\"2026-11-09T00:00:00Z\"}\n"
    );
}

#[test]
fn new_entries_are_tagged_and_old_ones_never_are() {
    let (dir, store) = store_with(4);
    let mut none = |_: &str| -> Result<String, String> { Err("unused".into()) };
    pass(&store, &mut none).unwrap();
    grow(&dir, 7, 4);
    let mut ask = |batch: &str| Ok(reply_for(&asked_ids(batch), |n| n == 5));
    let report = pass(&store, &mut ask).unwrap();
    assert_eq!((report.tagged, report.failed), (3, 0));
    let s = state(&dir);
    assert_eq!(s.tags.keys().copied().collect::<Vec<_>>(), [4, 5, 6]);
    assert!(s.tags[&5].rule && !s.tags[&4].rule);
}

#[test]
fn a_bad_reply_walks_the_ladder_twenty_five_one() {
    let (dir, store) = store_with(0);
    let mut none = |_: &str| -> Result<String, String> { Err("unused".into()) };
    pass(&store, &mut none).unwrap();
    grow(&dir, 6, 0);
    let sizes = RefCell::new(Vec::new());
    let mut ask = |batch: &str| {
        let ids = asked_ids(batch);
        sizes.borrow_mut().push(ids.len());
        if ids.len() > 1 {
            Ok("not json".to_string())
        } else {
            Ok(reply_for(&ids, |_| false))
        }
    };
    pass(&store, &mut ask).unwrap();
    assert_eq!(*sizes.borrow(), [6, 5, 1, 1, 1, 1, 1, 1]);
    assert_eq!(state(&dir).tags.len(), 6);
}

#[test]
fn valid_rows_of_a_partial_reply_are_kept_and_only_the_rest_drops_a_rung() {
    let (dir, store) = store_with(0);
    let mut none = |_: &str| -> Result<String, String> { Err("unused".into()) };
    pass(&store, &mut none).unwrap();
    grow(&dir, 3, 0);
    let calls = RefCell::new(Vec::new());
    let mut ask = |batch: &str| {
        let ids = asked_ids(batch);
        calls.borrow_mut().push(ids.clone());
        let answered: Vec<usize> = ids.iter().copied().filter(|n| *n != 1).collect();
        Ok(reply_for(&answered, |_| false))
    };
    pass(&store, &mut ask).unwrap();
    assert_eq!(*calls.borrow(), [vec![0, 1, 2], vec![1]]);
}

#[test]
fn an_entry_failing_alone_gets_a_failed_row_then_a_skipped_row_then_is_left_alone() {
    let (dir, store) = store_with(0);
    let mut none = |_: &str| -> Result<String, String> { Err("unused".into()) };
    pass(&store, &mut none).unwrap();
    grow(&dir, 2, 0);
    let mut ask = |batch: &str| {
        let ids: Vec<usize> = asked_ids(batch).into_iter().filter(|n| *n != 1).collect();
        Ok(reply_for(&ids, |_| false))
    };
    let first = pass(&store, &mut ask).unwrap();
    assert_eq!(first.failed, 1);
    let s = state(&dir);
    assert_eq!(s.failures.get(&1), Some(&1));
    assert!(s.skipped.is_empty() && !s.tags.contains_key(&1));
    pass(&store, &mut ask).unwrap();
    let s = state(&dir);
    assert!(s.skipped.contains(&1) && !s.tags.contains_key(&1));
    let mut never = |_: &str| -> Result<String, String> { panic!("no call expected") };
    pass(&store, &mut never).unwrap();
}

#[test]
fn a_model_error_ends_the_pass_and_writes_no_failure_rows() {
    let (dir, store) = store_with(0);
    let mut none = |_: &str| -> Result<String, String> { Err("unused".into()) };
    pass(&store, &mut none).unwrap();
    grow(&dir, 3, 0);
    let mut down = |_: &str| -> Result<String, String> { Err("exit 1".into()) };
    assert!(pass(&store, &mut down).is_err());
    let s = state(&dir);
    assert!(s.failures.is_empty() && s.tags.is_empty());
}

#[test]
fn corrupt_entries_are_never_asked() {
    let (dir, store) = store_with(0);
    let mut none = |_: &str| -> Result<String, String> { Err("unused".into()) };
    pass(&store, &mut none).unwrap();
    grow(&dir, 1, 0);
    let mut file = OpenOptions::new()
        .append(true)
        .open(dir.path().join("log.jsonl"))
        .unwrap();
    writeln!(file, "not json").unwrap();
    grow(&dir, 3, 2);
    let asked = RefCell::new(Vec::new());
    let mut ask = |batch: &str| {
        asked.borrow_mut().extend(asked_ids(batch));
        Ok(reply_for(&asked_ids(batch), |_| false))
    };
    pass(&store, &mut ask).unwrap();
    assert_eq!(*asked.borrow(), [0, 2]);
}

#[test]
fn a_held_tag_lock_makes_the_pass_exit_busy_without_a_call() {
    let (dir, store) = store_with(2);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.path().join("TAG"))
        .unwrap();
    lock.lock().unwrap();
    let mut never = |_: &str| -> Result<String, String> { panic!("no call expected") };
    assert!(matches!(store.run_pass(&now, &mut never), Ok(Pass::Busy)));
    assert!(!dir.path().join(TAGS_FILE).exists());
}

#[test]
fn a_torn_last_row_is_cut_off_by_the_next_append() {
    let (dir, store) = store_with(1);
    let mut none = |_: &str| -> Result<String, String> { Err("unused".into()) };
    pass(&store, &mut none).unwrap();
    let mut file = OpenOptions::new()
        .append(true)
        .open(dir.path().join(TAGS_FILE))
        .unwrap();
    write!(file, "{{\"v\":1,\"n\":9,\"ru").unwrap();
    grow(&dir, 2, 1);
    let mut ask = |batch: &str| Ok(reply_for(&asked_ids(batch), |_| true));
    pass(&store, &mut ask).unwrap();
    let s = state(&dir);
    assert_eq!(s.tags.keys().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(s.start, Some(1));
}

#[test]
fn entries_appended_during_a_pass_are_picked_up_by_the_same_pass() {
    let (dir, store) = store_with(0);
    let mut none = |_: &str| -> Result<String, String> { Err("unused".into()) };
    pass(&store, &mut none).unwrap();
    grow(&dir, 2, 0);
    let grown = RefCell::new(false);
    let mut ask = |batch: &str| {
        if !grown.replace(true) {
            grow(&dir, 3, 2);
        }
        Ok(reply_for(&asked_ids(batch), |_| false))
    };
    pass(&store, &mut ask).unwrap();
    assert_eq!(state(&dir).tags.len(), 3);
}

#[test]
fn roles_in_the_batch_come_from_the_log() {
    let (_dir, store) = store_with(1);
    let entries = store.read_entries().unwrap();
    assert_eq!(entries[0].role, Role::User);
}

#[test]
fn with_tags_reads_the_ids_for_a_tagged_request_only() {
    let dir = tempfile::tempdir().unwrap();
    let row = r#"{"v":1,"n":4,"rule":true,"decision":false,"approval":false,"model":"m","prompt":"v1","at":"t"}"#;
    fs::write(dir.path().join(TAGS_FILE), format!("{row}\n")).unwrap();
    let tagged = ralph_core::chat_memory::parse_search_args(
        &serde_json::json!({"query": "x", "tag": "rule"}),
    )
    .unwrap();
    let expected = tagged.clone().with_tag_ids(BTreeSet::from([4]));
    assert_eq!(with_tags(dir.path(), &tagged).unwrap(), expected);
    let plain =
        ralph_core::chat_memory::parse_search_args(&serde_json::json!({"query": "x"})).unwrap();
    assert_eq!(with_tags(dir.path(), &plain).unwrap(), plain);
}
