mod memory;
mod schedules;
mod threads;
mod watches;
mod workers;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use memory::{encode_log, encode_tree, memory_entries, memory_summaries};
use std::path::{Path, PathBuf};
use threads::thread_files;
use workers::worker_files;

pub use schedules::schedule_specs;
#[cfg(test)]
pub use schedules::ScheduleKind;
pub use threads::thread_specs;
pub use watches::watch_specs;
#[cfg(test)]
pub use watches::WatchShape;
pub use workers::worker_specs;
#[cfg(test)]
pub use workers::WorkerShape;

pub const PINNED_EPOCH: i64 = 1_772_452_800;

pub const CONFIG_TOML: &str =
    "[theme]\nname = \"tokyo-night\"\n\n[chat]\nwrap_width = 100\ncell_output_limit = 40\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWrite {
    pub path: PathBuf,
    pub contents: String,
}

pub fn at(offset_secs: i64) -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(PINNED_EPOCH + offset_secs)
}

pub fn stamp(offset_secs: i64) -> String {
    at(offset_secs).to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn epoch(offset_secs: i64) -> u64 {
    u64::try_from(PINNED_EPOCH + offset_secs).unwrap_or_default()
}

pub fn file_plan(store: &Path) -> Result<Vec<FileWrite>, String> {
    let mut files = vec![
        FileWrite {
            path: store.join("log.jsonl"),
            contents: encode_log(&memory_entries())?,
        },
        FileWrite {
            path: store.join("tree.jsonl"),
            contents: encode_tree(&memory_summaries()?)?,
        },
        FileWrite {
            path: store.join("config.toml"),
            contents: CONFIG_TOML.to_string(),
        },
    ];
    for spec in thread_specs() {
        files.extend(thread_files(store, &spec)?);
    }
    for spec in worker_specs() {
        files.extend(worker_files(store, &spec)?);
    }
    Ok(files)
}

pub fn out_dir_ok(entry_count: Option<usize>) -> Result<(), String> {
    match entry_count {
        None | Some(0) => Ok(()),
        Some(count) => Err(format!(
            "the output directory has {count} entries; pass a missing or empty directory"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::collections::HashSet;

    pub(super) const STORE: &str = "/store";
    const WINDOW_SECS: i64 = 30 * 86_400;

    fn plan() -> Vec<FileWrite> {
        file_plan(Path::new(STORE)).unwrap()
    }

    fn within_window(time: chrono::DateTime<chrono::Utc>) -> bool {
        (time - at(0)).num_seconds().abs() <= WINDOW_SECS
    }

    fn texts() -> Vec<String> {
        let mut texts: Vec<String> = plan()
            .into_iter()
            .flat_map(|file| [file.path.to_string_lossy().into_owned(), file.contents])
            .collect();
        texts.extend(
            schedule_specs()
                .iter()
                .map(|spec| serde_json::to_string(&spec.task).unwrap()),
        );
        texts.extend(
            watch_specs()
                .iter()
                .map(|entry| serde_json::to_string(&entry.saved).unwrap()),
        );
        texts
    }

    #[test]
    fn the_pinned_instant_is_fixed() {
        assert_eq!(stamp(0), "2026-03-02T12:00:00Z");
        assert_eq!(at(0).timestamp(), PINNED_EPOCH);
        assert_eq!(at(3600) - at(0), Duration::hours(1));
        assert_eq!(stamp(-60), "2026-03-02T11:59:00Z");
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<String> = Vec::new();
        ids.extend(thread_specs().iter().map(|spec| spec.name.to_string()));
        ids.extend(schedule_specs().iter().map(|spec| spec.task.id.clone()));
        ids.extend(watch_specs().iter().map(|entry| entry.saved.id.clone()));
        ids.extend(worker_specs().iter().map(|spec| spec.id.to_string()));
        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        assert!(!ids.is_empty());
    }

    #[test]
    fn every_time_derives_from_the_pinned_instant() {
        for spec in schedule_specs() {
            assert!(within_window(spec.task.at));
            assert!(spec.task.deadline.is_none_or(within_window));
        }
        for entry in watch_specs() {
            assert!(within_window(entry.saved.started_at));
            assert!(entry.saved.timeout_deadline.is_none_or(within_window));
            assert!(entry.saved.wake_deadline.is_none_or(within_window));
        }
        for spec in thread_specs() {
            assert!(spec.created.abs() <= WINDOW_SECS);
        }
        for entry in memory_entries() {
            let parsed = chrono::DateTime::parse_from_rfc3339(&entry.at).unwrap();
            assert!(within_window(parsed.into()));
        }
    }

    #[test]
    fn the_plan_is_deterministic_with_unique_paths() {
        let first = plan();
        assert_eq!(first, plan());
        let paths: HashSet<&PathBuf> = first.iter().map(|file| &file.path).collect();
        assert_eq!(paths.len(), first.len());
        assert!(first.iter().all(|file| file.path.starts_with(STORE)));
    }

    #[test]
    fn the_plan_writes_memory_threads_and_config() {
        let plan = plan();
        let has = |name: &str| plan.iter().any(|file| file.path.ends_with(name));
        assert!(has("log.jsonl") && has("tree.jsonl") && has("thread.json"));
        assert!(plan
            .iter()
            .any(|file| file.path == Path::new(STORE).join("config.toml")
                && file.contents == CONFIG_TOML));
    }

    #[test]
    fn no_text_has_private_looking_content() {
        let texts = texts();
        assert!(texts.len() > 20);
        for text in texts {
            for banned in ["/Users/", "@", "sk-", "token"] {
                assert!(!text.contains(banned), "{banned} in {text}");
            }
        }
    }

    #[test]
    fn only_a_missing_or_empty_out_dir_is_ok() {
        assert!(out_dir_ok(None).is_ok() && out_dir_ok(Some(0)).is_ok());
        assert!(out_dir_ok(Some(1)).is_err() && out_dir_ok(Some(40)).is_err());
    }
}
