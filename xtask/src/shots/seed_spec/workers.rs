use super::{epoch, FileWrite};
use ralph_core::worker::{
    activity_path, blocked_path, encode_spawn_record, gone_path, spawn_record_path,
    unreported_path, waiting_path, ActivityMark, ActivityState, HarnessId, SpawnRecord, Ticket,
    AGY_OWNER_DIR, CLAUDE_OWNER_DIR, CODEX_OWNER_DIR, CURSOR_OWNER_DIR, OPENCODE_OWNER_DIR,
};
use std::path::{Path, PathBuf};

const BLOCKED_MARKER: &str = r#"{"request_id":"req-1"}"#;
const GONE_NOTE: &str = "left";

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

#[cfg(test)]
mod tests {
    use super::super::tests::STORE;
    use super::*;
    use crate::shots::seed_spec::PINNED_EPOCH;
    use ralph_core::worker::{decode_spawn_record, encode_spawn_record, SpawnRecord, Ticket};
    use std::collections::HashSet;

    #[test]
    fn all_lists_every_worker_shape_variant_once() {
        let index = |value: WorkerShape| match value {
            WorkerShape::Running => 0,
            WorkerShape::Background => 1,
            WorkerShape::Idle => 2,
            WorkerShape::Failed => 3,
            WorkerShape::Blocked => 4,
            WorkerShape::Waiting => 5,
            WorkerShape::Unreported => 6,
            WorkerShape::Gone => 7,
        };
        let indexes: Vec<usize> = WorkerShape::ALL.iter().map(|value| index(*value)).collect();
        assert_eq!(indexes, (0..8).collect::<Vec<_>>());
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
}
