use chrono::{DateTime, Duration, SecondsFormat, Utc};
use ralph_core::chat_memory::{encode_entry, encode_record, NewEntry, Node, Role};
use ralph_core::chat_thread::{AutoMode, Merged, Parked, ThreadMeta, ThreadName, THREAD_FILE};
use ralph_core::schedule::{Task, TaskKind, TaskStatus, SCHEDULE_ID_PREFIX};
use ralph_core::schedule_repeat::Repeat;
use ralph_core::watch::WatchKind;
use ralph_core::watch_store::SavedWatch;
use ralph_core::worker::{
    activity_path, blocked_path, encode_spawn_record, gone_path, spawn_record_path,
    unreported_path, waiting_path, ActivityMark, ActivityState, HarnessId, SpawnRecord, Ticket,
    AGY_OWNER_DIR, CLAUDE_OWNER_DIR, CODEX_OWNER_DIR, CURSOR_OWNER_DIR, OPENCODE_OWNER_DIR,
};
use ralph_server::chat::thread::thread_dir;
use std::path::{Path, PathBuf};

pub const PINNED_EPOCH: i64 = 1_772_452_800;

pub const CONFIG_TOML: &str =
    "[theme]\nname = \"tokyo-night\"\n\n[chat]\nwrap_width = 100\ncell_output_limit = 40\n";

const MERGE_DIGEST: &str = "Demo digest: the work is done and the notes are in main.";
const BLOCKED_MARKER: &str = r#"{"request_id":"req-1"}"#;
const GONE_NOTE: &str = "left";

const TOPICS: [&str; 20] = [
    "tea brewing times",
    "garden planning",
    "a bicycle gear table",
    "bread starter schedule",
    "a reading list",
    "a packing checklist",
    "a chess opening note",
    "a recipe for lentil soup",
    "a plan for a quiet weekend",
    "a map of local parks",
    "a birdwatching log",
    "a jar of pickles",
    "a study timetable",
    "a list of board games",
    "a plan for a small shelf",
    "a note on cloud shapes",
    "a walking route by the river",
    "a list of herbs",
    "a plan for a picnic",
    "a sketch of a paper boat",
];

const SUMMARY_RANGES: [(usize, usize); 6] =
    [(0, 8), (8, 16), (16, 24), (24, 32), (0, 16), (16, 32)];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleKind {
    Once,
    Every,
    Daily,
    Due,
    Late,
    Running,
    Proposed,
    WorkerTimeout,
}

#[cfg(test)]
impl ScheduleKind {
    pub const ALL: [ScheduleKind; 8] = [
        Self::Once,
        Self::Every,
        Self::Daily,
        Self::Due,
        Self::Late,
        Self::Running,
        Self::Proposed,
        Self::WorkerTimeout,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleSpec {
    pub kind: ScheduleKind,
    pub task: Task,
}

fn command(text: &str) -> TaskKind {
    TaskKind::Command {
        command: text.to_string(),
    }
}

fn brief(text: &str) -> TaskKind {
    TaskKind::Worker {
        brief: text.to_string(),
    }
}

fn pending(name: &str, kind: TaskKind, offset: i64) -> Task {
    Task {
        id: format!("{SCHEDULE_ID_PREFIX}{name}"),
        kind,
        follow_up: None,
        at: at(offset),
        timeout_secs: None,
        status: TaskStatus::Pending,
        late: false,
        deadline: None,
        repeat: None,
    }
}

pub fn schedule_specs() -> Vec<ScheduleSpec> {
    let spec = |kind, task| ScheduleSpec { kind, task };
    vec![
        spec(
            ScheduleKind::Once,
            pending("demo-once", command("ls demo"), 7_200),
        ),
        spec(
            ScheduleKind::Every,
            Task {
                repeat: Some(Repeat::Every { every_secs: 3_600 }),
                ..pending("demo-every", command("sh demo-check.sh"), 3_600)
            },
        ),
        spec(
            ScheduleKind::Daily,
            Task {
                repeat: Some(Repeat::Daily {
                    daily: "02:00".to_string(),
                }),
                ..pending(
                    "demo-daily",
                    brief("Write a short made-up status note."),
                    50_400,
                )
            },
        ),
        spec(
            ScheduleKind::Due,
            pending("demo-due", command("ls demo-due"), -300),
        ),
        spec(
            ScheduleKind::Late,
            Task {
                late: true,
                ..pending("demo-late", command("ls demo-late"), -7_200)
            },
        ),
        spec(
            ScheduleKind::Running,
            Task {
                status: TaskStatus::Running,
                timeout_secs: Some(3_600),
                deadline: Some(at(1_800)),
                ..pending("demo-running", command("sh demo-long.sh"), -1_800)
            },
        ),
        spec(
            ScheduleKind::Proposed,
            Task {
                status: TaskStatus::Proposed,
                follow_up: Some("Demo follow-up: read the result.".to_string()),
                ..pending("demo-proposed", command("ls demo-next"), 86_400)
            },
        ),
        spec(
            ScheduleKind::WorkerTimeout,
            Task {
                timeout_secs: Some(7_200),
                deadline: Some(at(9_000)),
                ..pending("demo-worker", brief("Tidy the made-up notes."), 1_800)
            },
        ),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchShape {
    Plain,
    Timeout,
    Wake,
    Persistent,
}

#[cfg(test)]
impl WatchShape {
    pub const ALL: [WatchShape; 4] = [Self::Plain, Self::Timeout, Self::Wake, Self::Persistent];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEntry {
    pub shape: WatchShape,
    pub saved: SavedWatch,
}

fn watch(name: &str, command: &str, started: i64) -> SavedWatch {
    SavedWatch {
        id: format!("watch-{name}"),
        command: command.to_string(),
        follow_up: None,
        kind: WatchKind::Normal,
        timeout_secs: None,
        started_at: at(started),
        timeout_deadline: None,
        wake_deadline: None,
    }
}

pub fn watch_specs() -> Vec<WatchEntry> {
    let entry = |shape, saved| WatchEntry { shape, saved };
    vec![
        entry(
            WatchShape::Plain,
            watch("plain", "tail -f demo.log", -3_600),
        ),
        entry(
            WatchShape::Timeout,
            SavedWatch {
                timeout_secs: Some(1_800),
                timeout_deadline: Some(at(1_200)),
                ..watch("timeout", "sh demo-poll.sh", -600)
            },
        ),
        entry(
            WatchShape::Wake,
            SavedWatch {
                follow_up: Some("Demo follow-up: read the log.".to_string()),
                wake_deadline: Some(at(900)),
                ..watch("wake", "tail -f demo-wake.log", -1_200)
            },
        ),
        entry(
            WatchShape::Persistent,
            SavedWatch {
                kind: WatchKind::Persistent,
                ..watch("persistent", "tail -f demo-keep.log", -7_200)
            },
        ),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerShape {
    Running,
    Background,
    Idle,
    Failed,
    Blocked,
    Waiting,
    Unreported,
    Gone,
}

#[cfg(test)]
impl WorkerShape {
    pub const ALL: [WorkerShape; 8] = [
        Self::Running,
        Self::Background,
        Self::Idle,
        Self::Failed,
        Self::Blocked,
        Self::Waiting,
        Self::Unreported,
        Self::Gone,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Background => "background",
            Self::Idle => "idle",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Waiting => "waiting",
            Self::Unreported => "unreported",
            Self::Gone => "gone",
        }
    }
}

impl WorkerShape {
    fn activity(self) -> Option<ActivityState> {
        match self {
            Self::Running => Some(ActivityState::Running),
            Self::Background => Some(ActivityState::Background),
            Self::Idle => Some(ActivityState::Idle),
            Self::Failed => Some(ActivityState::Failed),
            Self::Blocked | Self::Waiting | Self::Unreported | Self::Gone => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSpec {
    pub id: &'static str,
    pub owner_dir: &'static str,
    pub harness: HarnessId,
    pub shape: WorkerShape,
    pub ticket: Option<&'static str>,
    pub offset: i64,
}

pub fn worker_specs() -> Vec<WorkerSpec> {
    let spec = |id, owner_dir, harness, shape, ticket, offset| WorkerSpec {
        id,
        owner_dir,
        harness,
        shape,
        ticket,
        offset,
    };
    vec![
        spec(
            "demo-running",
            CLAUDE_OWNER_DIR,
            HarnessId::Claude,
            WorkerShape::Running,
            Some("DEMO-101"),
            -3_000,
        ),
        spec(
            "demo-background",
            CODEX_OWNER_DIR,
            HarnessId::Codex,
            WorkerShape::Background,
            Some("DEMO-102"),
            -2_800,
        ),
        spec(
            "demo-idle",
            CURSOR_OWNER_DIR,
            HarnessId::Cursor,
            WorkerShape::Idle,
            None,
            -2_600,
        ),
        spec(
            "demo-failed",
            OPENCODE_OWNER_DIR,
            HarnessId::Opencode,
            WorkerShape::Failed,
            Some("DEMO-103"),
            -2_400,
        ),
        spec(
            "demo-blocked",
            AGY_OWNER_DIR,
            HarnessId::Agy,
            WorkerShape::Blocked,
            Some("DEMO-104"),
            -2_200,
        ),
        spec(
            "demo-waiting",
            CLAUDE_OWNER_DIR,
            HarnessId::Claude,
            WorkerShape::Waiting,
            None,
            -2_000,
        ),
        spec(
            "demo-unreported",
            CODEX_OWNER_DIR,
            HarnessId::Codex,
            WorkerShape::Unreported,
            Some("DEMO-105"),
            -1_800,
        ),
        spec(
            "demo-gone",
            CURSOR_OWNER_DIR,
            HarnessId::Cursor,
            WorkerShape::Gone,
            None,
            -1_600,
        ),
    ]
}

pub fn worker_files(store: &Path, spec: &WorkerSpec) -> Result<Vec<FileWrite>, String> {
    let file = |path: PathBuf, contents: &str| FileWrite {
        path,
        contents: contents.to_string(),
    };
    let record = SpawnRecord::new(
        spec.ticket.map(|key| Ticket {
            key: key.to_string(),
            tracker: None,
        }),
        None,
        spec.harness,
        epoch(spec.offset),
    );
    let mut files = vec![
        file(store.join(spec.owner_dir).join(spec.id), ""),
        file(
            spawn_record_path(store, spec.id),
            &encode_spawn_record(&record).map_err(|error| error.to_string())?,
        ),
    ];
    if let Some(state) = spec.shape.activity() {
        let mark = ActivityMark {
            at: epoch(spec.offset),
            state,
        };
        files.push(file(activity_path(store, spec.id), &mark.render()));
    }
    match spec.shape {
        WorkerShape::Blocked => files.push(file(blocked_path(store, spec.id), BLOCKED_MARKER)),
        WorkerShape::Waiting => files.push(file(waiting_path(store, spec.id), "")),
        WorkerShape::Unreported => files.push(file(unreported_path(store, spec.id), "")),
        WorkerShape::Gone => files.push(file(gone_path(store, spec.id), GONE_NOTE)),
        WorkerShape::Running
        | WorkerShape::Background
        | WorkerShape::Idle
        | WorkerShape::Failed => {}
    }
    Ok(files)
}

pub fn memory_entries() -> Vec<NewEntry> {
    TOPICS
        .iter()
        .zip(0_i64..)
        .flat_map(|(topic, index)| {
            let asked = -(20 - index) * 3_600;
            [
                NewEntry {
                    role: Role::User,
                    at: stamp(asked),
                    text: format!("Demo question {}: tell me about {topic}.", index + 1),
                },
                NewEntry {
                    role: Role::Assistant,
                    at: stamp(asked + 60),
                    text: format!(
                        "Demo answer {}: here is a short made-up note about {topic}.",
                        index + 1
                    ),
                },
            ]
        })
        .collect()
}

pub fn memory_summaries() -> Result<Vec<(Node, String)>, String> {
    SUMMARY_RANGES
        .iter()
        .map(|&(lo, hi)| {
            let text = format!(
                "Demo summary of entries {lo} to {}: short made-up notes on {} and {}.",
                hi - 1,
                TOPICS[lo / 2],
                TOPICS[(hi - 1) / 2]
            );
            Node::new(lo, hi).map(|node| (node, text))
        })
        .collect()
}

pub fn encode_log(entries: &[NewEntry]) -> Result<String, String> {
    entries.iter().map(encode_entry).collect()
}

pub fn encode_tree(nodes: &[(Node, String)]) -> Result<String, String> {
    nodes
        .iter()
        .map(|(node, text)| encode_record(*node, text))
        .collect()
}

fn thread_files(store: &Path, spec: &ThreadSpec) -> Result<Vec<FileWrite>, String> {
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
    use ralph_core::chat_memory::Role;
    use ralph_core::schedule::{TaskKind, TaskStatus};
    use ralph_core::schedule_repeat::Repeat;
    use ralph_core::watch::WatchKind;
    use ralph_core::worker::{decode_spawn_record, encode_spawn_record, SpawnRecord, Ticket};
    use std::collections::HashSet;

    const STORE: &str = "/store";
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

    #[test]
    fn every_schedule_kind_has_one_task() {
        let specs = schedule_specs();
        for kind in ScheduleKind::ALL {
            assert_eq!(
                specs.iter().filter(|spec| spec.kind == kind).count(),
                1,
                "{kind:?}"
            );
        }
        assert_eq!(specs.len(), ScheduleKind::ALL.len());
    }

    #[test]
    fn schedule_tasks_have_the_shape_of_their_kind() {
        for spec in schedule_specs() {
            let task = &spec.task;
            match spec.kind {
                ScheduleKind::Once => {
                    assert!(task.repeat.is_none() && task.at > at(0));
                    assert_eq!(task.status, TaskStatus::Pending);
                }
                ScheduleKind::Every => assert!(matches!(task.repeat, Some(Repeat::Every { .. }))),
                ScheduleKind::Daily => assert!(matches!(task.repeat, Some(Repeat::Daily { .. }))),
                ScheduleKind::Due => {
                    assert!(task.at <= at(0) && !task.late);
                    assert_eq!(task.status, TaskStatus::Pending);
                }
                ScheduleKind::Late => {
                    assert!(task.late && task.at < at(0));
                }
                ScheduleKind::Running => {
                    assert_eq!(task.status, TaskStatus::Running);
                    assert!(task.deadline.is_some());
                }
                ScheduleKind::Proposed => assert_eq!(task.status, TaskStatus::Proposed),
                ScheduleKind::WorkerTimeout => {
                    assert!(matches!(task.kind, TaskKind::Worker { .. }));
                    assert!(task.timeout_secs.is_some() && task.deadline.is_some());
                }
            }
        }
    }

    #[test]
    fn every_watch_shape_has_one_watch() {
        let specs = watch_specs();
        for shape in WatchShape::ALL {
            assert_eq!(
                specs.iter().filter(|entry| entry.shape == shape).count(),
                1,
                "{shape:?}"
            );
        }
        assert_eq!(specs.len(), WatchShape::ALL.len());
    }

    #[test]
    fn watches_have_the_shape_of_their_kind() {
        for entry in watch_specs() {
            let saved = &entry.saved;
            match entry.shape {
                WatchShape::Plain => {
                    assert_eq!(saved.kind, WatchKind::Normal);
                    assert!(saved.timeout_deadline.is_none() && saved.wake_deadline.is_none());
                }
                WatchShape::Timeout => {
                    assert!(saved.timeout_secs.is_some() && saved.timeout_deadline.is_some());
                }
                WatchShape::Wake => assert!(saved.wake_deadline.is_some()),
                WatchShape::Persistent => assert_eq!(saved.kind, WatchKind::Persistent),
            }
        }
    }

    #[test]
    fn every_worker_shape_has_one_worker() {
        let specs = worker_specs();
        for shape in WorkerShape::ALL {
            assert_eq!(
                specs.iter().filter(|spec| spec.shape == shape).count(),
                1,
                "{shape:?}"
            );
        }
        assert_eq!(specs.len(), WorkerShape::ALL.len());
    }

    #[test]
    fn worker_shapes_have_distinct_labels() {
        let labels: HashSet<&str> = WorkerShape::ALL.iter().map(|shape| shape.label()).collect();
        assert_eq!(labels.len(), WorkerShape::ALL.len());
        assert!(!labels.contains(""));
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
    fn memory_has_about_twenty_turns_that_alternate() {
        let entries = memory_entries();
        assert_eq!(entries.len(), 40);
        for (index, entry) in entries.iter().enumerate() {
            let expected = if index % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            assert_eq!(entry.role, expected);
            assert!(!entry.text.is_empty());
        }
        let times: Vec<&String> = entries.iter().map(|entry| &entry.at).collect();
        let mut sorted = times.clone();
        sorted.sort();
        assert_eq!(times, sorted);
    }

    #[test]
    fn memory_summaries_are_valid_blocks_inside_the_log() {
        let summaries = memory_summaries().unwrap();
        assert!(!summaries.is_empty());
        let total = memory_entries().len();
        let unique: HashSet<Node> = summaries.iter().map(|(node, _)| *node).collect();
        assert_eq!(unique.len(), summaries.len());
        for (node, text) in &summaries {
            assert!(node.hi <= total);
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn the_encoded_log_and_tree_read_back() {
        let entries = memory_entries();
        let (parsed, warnings) =
            ralph_core::chat_memory::parse_log(encode_log(&entries).unwrap().as_bytes());
        assert!(warnings.is_empty());
        assert_eq!(parsed.len(), entries.len());
        assert_eq!(parsed[3].text, entries[3].text);
        let summaries = memory_summaries().unwrap();
        let (tree, warnings) =
            ralph_core::chat_memory::parse_tree(encode_tree(&summaries).unwrap().as_bytes());
        assert!(warnings.is_empty());
        assert_eq!(tree.len(), summaries.len());
    }

    #[test]
    fn spawn_records_round_trip_through_decode() {
        let store = Path::new(STORE);
        for spec in worker_specs() {
            let files = worker_files(store, &spec).unwrap();
            let path = ralph_core::worker::spawn_record_path(store, spec.id);
            let file = files.iter().find(|file| file.path == path).unwrap();
            let expected = SpawnRecord::new(
                spec.ticket.map(|key| Ticket {
                    key: key.to_string(),
                    tracker: None,
                }),
                None,
                spec.harness,
                u64::try_from(PINNED_EPOCH + spec.offset).unwrap(),
            );
            assert_eq!(decode_spawn_record(&file.contents), Some(expected.clone()));
            assert_eq!(file.contents, encode_spawn_record(&expected).unwrap());
        }
    }

    #[test]
    fn worker_files_mark_each_state() {
        let store = Path::new(STORE);
        for spec in worker_specs() {
            let files = worker_files(store, &spec).unwrap();
            let has = |path: PathBuf| files.iter().any(|file| file.path == path);
            assert!(has(store.join(spec.owner_dir).join(spec.id)));
            assert_eq!(
                has(ralph_core::worker::gone_path(store, spec.id)),
                spec.shape == WorkerShape::Gone
            );
            assert_eq!(
                has(ralph_core::worker::blocked_path(store, spec.id)),
                spec.shape == WorkerShape::Blocked
            );
            assert_eq!(
                has(ralph_core::worker::waiting_path(store, spec.id)),
                spec.shape == WorkerShape::Waiting
            );
            assert_eq!(
                has(ralph_core::worker::unreported_path(store, spec.id)),
                spec.shape == WorkerShape::Unreported
            );
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
