use crate::warn::{warn, warn_if_err};
use ralph_core::schedule::Task;
use ralph_core::watch_store::file_name;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ScheduleStore {
    dir: PathBuf,
}

impl ScheduleStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn save(&self, task: &Task) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let name = file_name(&task.id);
        let tmp = self.dir.join(format!(".{name}"));
        fs::write(&tmp, serde_json::to_vec(task)?)?;
        fs::rename(tmp, self.dir.join(name))
    }

    pub fn remove(&self, id: &str) {
        remove_file(&self.dir.join(file_name(id)));
    }

    pub fn load_all(&self) -> Vec<Task> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                warn(format!("reading scheduled tasks: {e}"));
                return Vec::new();
            }
        };
        entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| is_record(path))
            .filter_map(|path| load(&path))
            .collect()
    }
}

fn is_record(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "json")
        && path
            .file_name()
            .is_some_and(|name| !name.to_string_lossy().starts_with('.'))
}

fn load(path: &Path) -> Option<Task> {
    let parsed = fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()));
    match parsed {
        Ok(saved) => Some(saved),
        Err(e) => {
            warn(format!(
                "discarding unreadable scheduled task {}: {e}",
                path.display()
            ));
            remove_file(path);
            None
        }
    }
}

fn remove_file(path: &Path) {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            warn_if_err(Err::<(), _>(e), format!("removing {}", path.display()));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ralph_core::schedule::{TaskKind, TaskStatus};

    fn task(id: &str) -> Task {
        Task {
            id: id.to_string(),
            kind: TaskKind::Command {
                command: "echo hi".to_string(),
            },
            follow_up: None,
            at: Utc::now(),
            timeout_secs: None,
            status: TaskStatus::Proposed,
            late: false,
        }
    }

    fn ids(store: &ScheduleStore) -> Vec<String> {
        let mut ids: Vec<String> = store.load_all().into_iter().map(|t| t.id).collect();
        ids.sort();
        ids
    }

    #[test]
    fn saved_records_load_back_and_can_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("schedule"));
        let record = task("a");
        store.save(&record).unwrap();
        store.save(&task("b")).unwrap();
        assert_eq!(ids(&store), vec!["a", "b"]);
        store.remove("b");
        store.remove("missing");
        assert_eq!(store.load_all(), vec![record]);
    }

    #[test]
    fn saving_again_replaces_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        store.save(&task("a")).unwrap();
        let mut changed = task("a");
        changed.status = TaskStatus::Pending;
        store.save(&changed).unwrap();
        assert_eq!(store.load_all(), vec![changed]);
    }

    #[test]
    fn missing_directory_loads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("none"));
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn corrupt_records_are_deleted_and_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        store.save(&task("ok")).unwrap();
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "{nope").unwrap();
        assert_eq!(store.load_all().len(), 1);
        assert!(!bad.exists());
    }

    #[test]
    fn temporary_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().to_path_buf());
        fs::write(dir.path().join(".partial.json"), "{").unwrap();
        assert!(store.load_all().is_empty());
        assert!(dir.path().join(".partial.json").exists());
    }
}
