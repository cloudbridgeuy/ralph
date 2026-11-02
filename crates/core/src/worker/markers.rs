use super::{
    Scope, ACTIVITY_DIR, BLOCKED_DIR, GONE_DIR, READY_DIR, REPORTED_DIR, SENT_DIR, UNREPORTED_DIR,
    WAITING_DIR,
};
use std::path::{Path, PathBuf};

pub fn ready_path(store: &Path, id: &str) -> PathBuf {
    store.join(READY_DIR).join(id)
}

pub fn reported_path(store: &Path, id: &str) -> PathBuf {
    store.join(REPORTED_DIR).join(id)
}

pub fn blocked_path(store: &Path, id: &str) -> PathBuf {
    store.join(BLOCKED_DIR).join(id)
}

pub fn waiting_path(store: &Path, id: &str) -> PathBuf {
    store.join(WAITING_DIR).join(id)
}

pub fn unreported_path(store: &Path, id: &str) -> PathBuf {
    store.join(UNREPORTED_DIR).join(id)
}

pub fn sent_path(store: &Path, id: &str) -> PathBuf {
    store.join(SENT_DIR).join(id)
}

pub fn activity_path(store: &Path, id: &str) -> PathBuf {
    store.join(ACTIVITY_DIR).join(id)
}

pub fn gone_path(store: &Path, id: &str) -> PathBuf {
    store.join(GONE_DIR).join(id)
}

const SERVER_GONE_PREFIX: &str = "scope server ";

pub fn render_gone(scope: &Scope, reason: &str) -> String {
    match scope {
        Scope::Worker => reason.to_string(),
        Scope::Server(name) => format!("{SERVER_GONE_PREFIX}{name}\n{reason}"),
    }
}

pub fn parse_gone(text: &str) -> (Scope, String) {
    let server = text
        .split_once('\n')
        .and_then(|(head, reason)| Some((head.strip_prefix(SERVER_GONE_PREFIX)?, reason)));
    match server {
        Some((name, reason)) => (Scope::Server(name.to_string()), reason.trim().to_string()),
        None => (Scope::Worker, text.trim().to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityState {
    Running,
    Background,
    Idle,
    Failed,
}

impl ActivityState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Background => "background",
            Self::Idle => "idle",
            Self::Failed => "failed",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        match label {
            "running" => Some(Self::Running),
            "background" => Some(Self::Background),
            "idle" => Some(Self::Idle),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn busy(self) -> bool {
        matches!(self, Self::Running | Self::Background)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityMark {
    pub at: u64,
    pub state: ActivityState,
}

impl ActivityMark {
    pub fn render(&self) -> String {
        format!("{} {}", self.at, self.state.label())
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (at, flag) = text.trim().split_once(' ')?;
        Some(Self {
            at: at.parse().ok()?,
            state: ActivityState::parse(flag)?,
        })
    }

    pub fn busy(&self) -> bool {
        self.state.busy()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerMarkers {
    pub ready: PathBuf,
    pub reported: PathBuf,
    pub blocked: PathBuf,
    pub waiting: PathBuf,
    pub unreported: PathBuf,
    pub sent: PathBuf,
    pub activity: PathBuf,
    pub gone: PathBuf,
}

impl WorkerMarkers {
    pub fn new(store: &Path, id: &str) -> Self {
        Self {
            ready: ready_path(store, id),
            reported: reported_path(store, id),
            blocked: blocked_path(store, id),
            waiting: waiting_path(store, id),
            unreported: unreported_path(store, id),
            sent: sent_path(store, id),
            activity: activity_path(store, id),
            gone: gone_path(store, id),
        }
    }
}

pub fn marker_paths(store: &Path, id: &str) -> [PathBuf; 8] {
    let WorkerMarkers {
        ready,
        reported,
        blocked,
        waiting,
        unreported,
        sent,
        activity,
        gone,
    } = WorkerMarkers::new(store, id);
    [
        ready, reported, blocked, waiting, unreported, sent, activity, gone,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_path_is_per_worker_in_the_store() {
        assert_eq!(
            ready_path(Path::new("/s"), "A"),
            PathBuf::from("/s/ready/A")
        );
    }

    #[test]
    fn reported_path_is_per_worker_in_the_store() {
        assert_eq!(
            reported_path(Path::new("/s"), "A"),
            PathBuf::from("/s/reported/A")
        );
    }

    #[test]
    fn sent_path_is_per_worker_in_the_store() {
        assert_eq!(sent_path(Path::new("/s"), "A"), PathBuf::from("/s/sent/A"));
    }

    #[test]
    fn marker_paths_cover_every_marker_dir_for_the_worker() {
        assert_eq!(
            marker_paths(Path::new("/s"), "A"),
            [
                PathBuf::from("/s/ready/A"),
                PathBuf::from("/s/reported/A"),
                PathBuf::from("/s/blocked/A"),
                PathBuf::from("/s/waiting/A"),
                PathBuf::from("/s/unreported/A"),
                PathBuf::from("/s/sent/A"),
                PathBuf::from("/s/activity/A"),
                PathBuf::from("/s/gone/A"),
            ]
        );
    }

    #[test]
    fn gone_marker_round_trips_both_scopes() {
        let server = Scope::Server("pid 7".into());
        assert_eq!(
            parse_gone(&render_gone(&server, "exited")),
            (server, "exited".into())
        );
        assert_eq!(
            parse_gone(&render_gone(&Scope::Worker, "boom")),
            (Scope::Worker, "boom".into())
        );
        assert_eq!(parse_gone("server x\nboom\n").0, Scope::Worker);
    }

    #[test]
    fn activity_mark_round_trips_and_rejects_garbage() {
        for state in [
            ActivityState::Running,
            ActivityState::Background,
            ActivityState::Idle,
            ActivityState::Failed,
        ] {
            let mark = ActivityMark { at: 42, state };
            assert_eq!(ActivityMark::parse(&mark.render()), Some(mark));
        }
        assert_eq!(ActivityMark::parse("42 paused"), None);
        assert_eq!(ActivityMark::parse("x running"), None);
        assert_eq!(ActivityMark::parse(""), None);
    }

    #[test]
    fn activity_mark_parses_background_and_failed() {
        assert_eq!(
            ActivityMark::parse("123 background").map(|mark| mark.state),
            Some(ActivityState::Background)
        );
        assert_eq!(
            ActivityMark::parse("123 failed").map(|mark| mark.state),
            Some(ActivityState::Failed)
        );
    }

    #[test]
    fn busy_is_running_or_background_only() {
        assert!(ActivityState::Running.busy());
        assert!(ActivityState::Background.busy());
        assert!(!ActivityState::Idle.busy());
        assert!(!ActivityState::Failed.busy());
    }
}
