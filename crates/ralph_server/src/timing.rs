use crate::outcome_log::open_log;
use crate::private_file::check_path;
use crate::warn::warn_if_err;
use crate::workers::usage_store;
use ralph_core::chat_memory::{encode_timing, timing_path, TimingKey, TimingRow};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub fn record(store: &Path, run: Option<&str>, build: impl FnOnce(&TimingKey) -> TimingRow) {
    let canonical = usage_store(store);
    let row = build(&TimingKey {
        store: &canonical,
        run,
        at: chrono::Utc::now(),
    });
    warn_if_err(append(store, &row), "recording chat timing");
}

fn append(store: &Path, row: &TimingRow) -> io::Result<()> {
    let line = encode_timing(row).map_err(io::Error::other)?;
    let path = timing_path(store);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    check_path(&path)?;
    open_log(&path)?.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_memory::{turn_row, TurnTiming};
    use std::fs;
    use std::time::Duration;

    fn row(key: &TimingKey) -> TimingRow {
        turn_row(key, Duration::from_millis(5), &TurnTiming::default(), true)
    }

    #[test]
    fn two_appends_make_two_json_lines() {
        let dir = tempfile::tempdir().unwrap();
        let key = TimingKey {
            store: "/s",
            run: Some("r"),
            at: chrono::Utc::now(),
        };
        append(dir.path(), &row(&key)).unwrap();
        append(dir.path(), &row(&key)).unwrap();
        let text = fs::read_to_string(timing_path(dir.path())).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().all(|line| line.contains("\"event\":\"turn\"")));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn record_stamps_the_canonical_store_and_the_run() {
        let dir = tempfile::tempdir().unwrap();
        record(dir.path(), Some("run-9"), row);
        let text = fs::read_to_string(timing_path(dir.path())).unwrap();
        let value: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(value["run"], "run-9");
        assert_eq!(value["store"], usage_store(dir.path()));
    }

    #[test]
    fn a_failed_write_is_swallowed_by_record() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        fs::write(&blocker, "").unwrap();
        assert!(append(
            &blocker,
            &row(&TimingKey {
                store: "/s",
                run: None,
                at: chrono::Utc::now(),
            })
        )
        .is_err());
        record(&blocker, None, row);
    }

    #[test]
    fn a_symlinked_log_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = timing_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        record(dir.path(), None, row);
        assert_eq!(fs::read_to_string(&target).unwrap(), "");
    }
}
