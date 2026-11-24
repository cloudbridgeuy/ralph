use super::*;
use ralph_core::chat_stage::{Item, Kind, NewItem};
use serde_json::json;

fn item(id: &str, kind: Kind) -> Item {
    Item {
        id: id.to_string(),
        kind,
        reference: None,
        text: "check the build".to_string(),
        created: "2026-01-01T00:00:00Z".to_string(),
        turn: 0,
    }
}

fn log_of(dir: &Path, rows: usize) -> Store {
    let store = Store::new(dir.to_path_buf());
    for _ in 0..rows {
        store.append(Role::User, "hi").unwrap();
    }
    store
}

#[test]
fn an_expired_item_leaves_the_stage_and_writes_an_audit_row_and_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let store = log_of(dir.path(), 60);
    let stage = Stage {
        items: vec![item("s1", Kind::Followup), item("s2", Kind::Decision)],
        last: 2,
    };
    save(dir.path(), &stage).unwrap();
    expire_due(&store, dir.path()).unwrap();
    let kept = load(dir.path()).unwrap();
    assert_eq!(kept.items, vec![item("s2", Kind::Decision)]);
    assert_eq!(kept.last, 2);
    let audit = fs::read_to_string(dir.path().join(AUDIT_FILE)).unwrap();
    let rows: Vec<AuditRow> = audit
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].actor, rows[0].op), (Actor::Expiry, Op::Expire));
    assert_eq!(rows[0].item.id, "s1");
    let (entries, _) = store.load_log_quiet().unwrap();
    assert_eq!(
        entries.last().unwrap().text,
        notice_record("Stage: s1 expired: check the build")
    );
}

#[test]
fn nothing_due_touches_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = log_of(dir.path(), 59);
    save(
        dir.path(),
        &Stage {
            items: vec![item("s1", Kind::Owed)],
            last: 1,
        },
    )
    .unwrap();
    expire_due(&store, dir.path()).unwrap();
    assert!(!dir.path().join(AUDIT_FILE).exists());
    assert_eq!(store.log_len().unwrap(), 59);
}

fn audit_rows(dir: &Path) -> Vec<AuditRow> {
    fs::read_to_string(dir.join(AUDIT_FILE))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn add_and_done_write_one_audit_row_each_and_ids_do_not_come_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = log_of(dir.path(), 2);
    let added = tool_add(
        &store,
        &json!({"kind": "decision", "text": "merge?", "ref": "GUZ-1"}),
    );
    assert_eq!(added, Ok("s1 added".to_string()));
    assert_eq!(load(dir.path()).unwrap().items[0].turn, 2);
    assert_eq!(
        tool_done(&store, &json!({"id": "s1", "note": "yes"})),
        Ok("s1 done".to_string())
    );
    assert_eq!(
        tool_add(&store, &json!({"kind": "owed", "text": "x"})),
        Ok("s2 added".to_string())
    );
    let rows = audit_rows(dir.path());
    let ops: Vec<_> = rows.iter().map(|row| (row.actor, row.op)).collect();
    assert_eq!(
        ops,
        [
            (Actor::Model, Op::Add),
            (Actor::Model, Op::Done),
            (Actor::Model, Op::Add)
        ]
    );
    assert_eq!(rows[1].note.as_deref(), Some("yes"));
}

#[test]
fn only_stage_tools_are_taken() {
    let dir = tempfile::tempdir().unwrap();
    let store = log_of(dir.path(), 0);
    let request = |tool: &str, args| SocketRequest {
        tool: tool.to_string(),
        args,
    };
    assert_eq!(call(&store, &request("workers", json!({}))), None);
    let added = call(
        &store,
        &request("stage_add", json!({"kind": "owed", "text": "x"})),
    );
    assert_eq!(added, Some(Ok("s1 added".to_string())));
}

#[test]
fn a_bad_call_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = log_of(dir.path(), 0);
    assert!(tool_done(&store, &json!({"id": "s7"})).is_err());
    assert!(tool_add(&store, &json!({"kind": "owed", "text": "a\nb"})).is_err());
    assert!(!dir.path().join(STAGE_FILE).exists());
    assert!(!dir.path().join(AUDIT_FILE).exists());
}

#[test]
fn a_user_add_edit_and_done_write_user_rows_and_edit_keeps_the_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = log_of(dir.path(), 4);
    let new = NewItem {
        kind: Kind::Owed,
        reference: None,
        text: "ping w1".to_string(),
    };
    assert_eq!(user_add(&store, new), Ok("s1 added".to_string()));
    assert_eq!(
        user_edit(&store, "s1", "ping w2"),
        Ok("s1 edited".to_string())
    );
    assert_eq!(user_done(&store, "s1"), Ok("s1 done".to_string()));
    let rows = audit_rows(dir.path());
    let ops: Vec<_> = rows.iter().map(|row| (row.actor, row.op)).collect();
    assert_eq!(
        ops,
        [
            (Actor::User, Op::Add),
            (Actor::User, Op::Edit),
            (Actor::User, Op::Done)
        ]
    );
    assert_eq!(rows[1].item.text, "ping w2");
    assert_eq!(rows[1].item.turn, 4);
    assert!(load(dir.path()).unwrap().items.is_empty());
}

#[test]
fn a_bad_user_edit_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = log_of(dir.path(), 0);
    assert!(user_edit(&store, "s3", "x").is_err());
    assert!(user_done(&store, "s3").is_err());
    assert!(!dir.path().join(STAGE_FILE).exists());
    assert!(!dir.path().join(AUDIT_FILE).exists());
}

#[test]
fn the_stage_lock_blocks_a_second_holder_until_it_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let inside = locked(dir.path(), || {
        let other = open_stage_lock(dir.path()).unwrap();
        Ok(other.try_lock().is_err())
    });
    assert_eq!(inside, Ok(true));
    let other = open_stage_lock(dir.path()).unwrap();
    assert!(other.try_lock().is_ok());
}
