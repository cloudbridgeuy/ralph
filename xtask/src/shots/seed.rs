use super::seed_spec::{file_plan, out_dir_ok, schedule_specs, watch_specs, FileWrite};
use super::BoxError;
use ralph_core::schedule_request::SCHEDULE_DIR;
use ralph_core::watch_store::WATCHES_DIR;
use ralph_server::schedule_store::ScheduleStore;
use ralph_server::watch_store::WatchStore;
use std::fs;
use std::io;
use std::path::Path;

fn entry_count(dir: &Path) -> Result<Option<usize>, BoxError> {
    match fs::read_dir(dir) {
        Ok(entries) => Ok(Some(entries.count())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("could not read {}: {error}", dir.display()).into()),
    }
}

fn write_file(file: &FileWrite) -> Result<(), BoxError> {
    if let Some(parent) = file.path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&file.path, &file.contents)
        .map_err(|error| format!("could not write {}: {error}", file.path.display()).into())
}

pub fn seed(out: &Path) -> Result<usize, BoxError> {
    out_dir_ok(entry_count(out)?)?;
    fs::create_dir_all(out)?;
    let files = file_plan(out)?;
    for file in &files {
        write_file(file)?;
    }
    let schedules = ScheduleStore::new(out.join(SCHEDULE_DIR));
    for spec in schedule_specs() {
        schedules.save(&spec.task)?;
    }
    let watches = WatchStore::new(out.join(WATCHES_DIR));
    for entry in watch_specs() {
        watches.save(&entry.saved)?;
    }
    Ok(files.len() + schedule_specs().len() + watch_specs().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shots::seed_spec::{
        thread_specs, worker_specs, ScheduleKind, WatchShape, WorkerShape,
    };
    use ralph_core::schedule_command::task_rows;
    use ralph_core::watch_command::watch_rows;
    use ralph_core::workers_view::worker_state_rows;
    use ralph_server::chat::memory::{list_threads, peek};
    use ralph_server::worker_facts::worker_facts;

    #[test]
    fn a_seeded_store_reads_back_in_every_list() {
        let out = tempfile::tempdir().unwrap();
        let store = out.path().join("fixture");
        let written = seed(&store).unwrap();
        assert!(written > 40);

        let tasks = ScheduleStore::new(store.join(SCHEDULE_DIR)).read_all();
        assert_eq!(task_rows(&tasks).len(), ScheduleKind::ALL.len());

        let saved = WatchStore::new(store.join(WATCHES_DIR)).read_all();
        assert_eq!(watch_rows(&saved).len(), WatchShape::ALL.len());

        let rows = worker_state_rows(&worker_facts(&store));
        let mut states: Vec<&str> = rows.iter().map(|row| row.state).collect();
        states.sort_unstable();
        let mut expected: Vec<&str> = WorkerShape::ALL.iter().map(|s| s.label()).collect();
        expected.sort_unstable();
        assert_eq!(states, expected);
        assert_eq!(rows.len(), worker_specs().len());

        let data = peek(&store).unwrap();
        assert_eq!(data.entries.len(), 40);
        assert!(!data.tree.is_empty());
        assert_eq!(list_threads(&store).unwrap().len(), thread_specs().len());
    }

    #[test]
    fn seeding_twice_into_the_same_dir_is_refused() {
        let out = tempfile::tempdir().unwrap();
        let store = out.path().join("fixture");
        seed(&store).unwrap();
        assert!(seed(&store).is_err());
    }

    #[test]
    fn two_seeds_write_equal_stores() {
        let out = tempfile::tempdir().unwrap();
        let (a, b) = (out.path().join("a"), out.path().join("b"));
        seed(&a).unwrap();
        seed(&b).unwrap();
        let read = |dir: &Path| fs::read_to_string(dir.join("log.jsonl")).unwrap();
        assert_eq!(read(&a), read(&b));
        let names = |dir: &Path| {
            let mut names: Vec<_> = fs::read_dir(dir.join(SCHEDULE_DIR))
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .collect();
            names.sort();
            names
        };
        assert_eq!(names(&a), names(&b));
    }
}
