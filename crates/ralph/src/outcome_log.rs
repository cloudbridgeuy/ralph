use crate::private_file::check_path;
use ralph_core::worker::{encode_event, workers_log_path, LifecycleEvent};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

pub(crate) fn append(store: &Path, event: &LifecycleEvent) -> io::Result<()> {
    let line = encode_event(event).map_err(io::Error::other)?;
    let path = workers_log_path(store);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    check_path(&path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use ralph_core::worker::{spawn_event, HarnessId, LogKey, SpawnFacts};

    fn event(worker_id: &str) -> LifecycleEvent {
        spawn_event(&SpawnFacts {
            key: LogKey {
                store: "/s",
                worker_id,
                at: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            },
            ticket: None,
            tracker: None,
            harness: HarnessId::Claude,
            model: None,
            base_sha: None,
            scheduled: false,
        })
    }

    #[test]
    fn two_appends_make_two_json_lines() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), &event("A")).unwrap();
        append(dir.path(), &event("B")).unwrap();
        let text = fs::read_to_string(workers_log_path(dir.path())).unwrap();
        let ids: Vec<String> = text
            .lines()
            .map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).unwrap();
                value["worker_id"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(ids, ["A", "B"]);
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn a_symlinked_log_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = workers_log_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(append(dir.path(), &event("A")).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "");
    }
}
