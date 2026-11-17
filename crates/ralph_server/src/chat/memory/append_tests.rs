use super::*;

#[test]
fn append_id_returns_the_index_of_the_row_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("s"));
    for expected in 0..3 {
        let id = store.append_id(Role::User, "row").unwrap();
        assert_eq!(id, expected);
        assert_eq!(id, store.log_len().unwrap() - 1);
    }
}

#[test]
fn append_id_counts_after_a_torn_tail_is_trimmed() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    store.append(Role::User, "one").unwrap();
    let mut log = OpenOptions::new()
        .append(true)
        .open(dir.path().join("log.jsonl"))
        .unwrap();
    log.write_all(b"{\"role\":\"us").unwrap();
    assert_eq!(store.append_id(Role::User, "two").unwrap(), 1);
    assert_eq!(store.log_len().unwrap(), 2);
}

#[test]
fn append_id_does_not_touch_the_tree_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    store.append(Role::User, "one").unwrap();
    fs::create_dir(dir.path().join("tree.jsonl")).unwrap();
    assert_eq!(store.append_id(Role::User, "two").unwrap(), 1);
    assert_eq!(store.append_id(Role::User, "three").unwrap(), 2);
}
