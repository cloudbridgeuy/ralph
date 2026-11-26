use super::memory::encode_log;
use super::{stamp, FileWrite};
use ralph_core::chat_memory::{NewEntry, Role};
use ralph_core::chat_thread::{AutoMode, Merged, Parked, ThreadMeta, ThreadName, THREAD_FILE};
use ralph_server::chat::thread::thread_dir;
use std::path::Path;

const MERGE_DIGEST: &str = "Demo digest: the work is done and the notes are in main.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Open,
    Parked,
    Merged,
    Closed,
    AutoMerge,
}

#[cfg(test)]
impl ThreadState {
    pub const ALL: [ThreadState; 5] = [
        Self::Open,
        Self::Parked,
        Self::Merged,
        Self::Closed,
        Self::AutoMerge,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSpec {
    pub name: &'static str,
    pub state: ThreadState,
    pub base: usize,
    pub created: i64,
    pub notes: &'static [&'static str],
}

pub fn thread_specs() -> Vec<ThreadSpec> {
    vec![
        ThreadSpec {
            name: "retry-backoff",
            state: ThreadState::Open,
            base: 14,
            created: -20_000,
            notes: &[
                "Try a slower retry.",
                "Demo reply: wait twice as long each time.",
                "Try a cap of one minute.",
            ],
        },
        ThreadSpec {
            name: "docs-outline",
            state: ThreadState::Parked,
            base: 18,
            created: -18_000,
            notes: &[
                "Draft an outline.",
                "Demo reply: three headings and a list.",
            ],
        },
        ThreadSpec {
            name: "lint-pass",
            state: ThreadState::Merged,
            base: 10,
            created: -30_000,
            notes: &["Run the lint pass.", "Demo reply: three small fixes."],
        },
        ThreadSpec {
            name: "old-sketch",
            state: ThreadState::Closed,
            base: 4,
            created: -40_000,
            notes: &["A sketch that was dropped."],
        },
        ThreadSpec {
            name: "cache-warmup",
            state: ThreadState::AutoMerge,
            base: 8,
            created: -9_000,
            notes: &[
                "Warm the cache first.",
                "Demo reply: start with the small files.",
            ],
        },
    ]
}

pub fn thread_meta(spec: &ThreadSpec) -> ThreadMeta {
    ThreadMeta {
        base: spec.base,
        created: stamp(spec.created),
        merged: (spec.state == ThreadState::Merged).then(|| Merged {
            at: stamp(spec.created + 3_600),
            digest: MERGE_DIGEST.to_string(),
            landed: Some(spec.base + 2),
        }),
        parked: (spec.state == ThreadState::Parked).then(|| Parked {
            at: stamp(spec.created + 1_800),
        }),
        auto: (spec.state == ThreadState::AutoMerge).then_some(AutoMode::Merge),
        model: None,
        closed: spec.state == ThreadState::Closed,
    }
}

fn thread_entries(spec: &ThreadSpec) -> Vec<NewEntry> {
    spec.notes
        .iter()
        .enumerate()
        .map(|(index, note)| NewEntry {
            role: if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            },
            at: stamp(spec.created + 60 * i64::try_from(index).unwrap_or_default()),
            text: (*note).to_string(),
        })
        .collect()
}

pub(super) fn thread_files(store: &Path, spec: &ThreadSpec) -> Result<Vec<FileWrite>, String> {
    let dir = thread_dir(store, &ThreadName::parse(spec.name)?);
    let meta = serde_json::to_string(&thread_meta(spec)).map_err(|error| error.to_string())?;
    Ok(vec![
        FileWrite {
            path: dir.join(THREAD_FILE),
            contents: meta,
        },
        FileWrite {
            path: dir.join("log.jsonl"),
            contents: encode_log(&thread_entries(spec))?,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::super::memory::memory_entries;
    use super::*;

    #[test]
    fn every_thread_state_has_one_thread() {
        let specs = thread_specs();
        for state in ThreadState::ALL {
            assert_eq!(
                specs.iter().filter(|spec| spec.state == state).count(),
                1,
                "{state:?}"
            );
        }
        assert_eq!(specs.len(), ThreadState::ALL.len());
    }

    #[test]
    fn thread_meta_matches_the_state() {
        for spec in thread_specs() {
            let meta = thread_meta(&spec);
            assert_eq!(meta.base, spec.base);
            assert_eq!(meta.created, stamp(spec.created));
            assert_eq!(
                meta.closed,
                spec.state == ThreadState::Closed,
                "{}",
                spec.name
            );
            assert_eq!(meta.parked.is_some(), spec.state == ThreadState::Parked);
            assert_eq!(meta.merged.is_some(), spec.state == ThreadState::Merged);
            assert_eq!(
                meta.auto,
                (spec.state == ThreadState::AutoMerge)
                    .then_some(ralph_core::chat_thread::AutoMode::Merge)
            );
        }
    }

    #[test]
    fn thread_names_parse_and_bases_fit_the_main_log() {
        let total = memory_entries().len();
        for spec in thread_specs() {
            assert!(ralph_core::chat_thread::ThreadName::parse(spec.name).is_ok());
            assert!((1..=3).contains(&spec.notes.len()), "{}", spec.name);
            assert!(spec.base <= total, "{}", spec.name);
        }
    }
}
