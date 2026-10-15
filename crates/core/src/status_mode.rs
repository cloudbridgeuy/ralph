use crate::chat_status::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Nap,
    Inbox,
    Work,
    Idle,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Nap => "NAP",
            Mode::Inbox => "INBOX",
            Mode::Work => "WORK",
            Mode::Idle => "IDLE",
        }
    }
}

pub fn mode(status: &Status) -> Mode {
    if status.napping {
        Mode::Nap
    } else if status.unread > 0 {
        Mode::Inbox
    } else if status.workers.iter().any(|w| w.is_running()) || !status.watches.is_empty() {
        Mode::Work
    } else {
        Mode::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_status::{WatchState, WorkerState};

    fn worker(state: &str) -> WorkerState {
        WorkerState {
            id: "w".into(),
            state: state.into(),
        }
    }

    fn watch() -> WatchState {
        WatchState {
            id: "t".into(),
            left: None,
        }
    }

    #[test]
    fn empty_status_is_idle() {
        assert_eq!(mode(&Status::default()), Mode::Idle);
    }

    #[test]
    fn napping_is_nap() {
        let status = Status {
            napping: true,
            ..Status::default()
        };
        assert_eq!(mode(&status), Mode::Nap);
    }

    #[test]
    fn unread_is_inbox() {
        let status = Status {
            unread: 1,
            ..Status::default()
        };
        assert_eq!(mode(&status), Mode::Inbox);
    }

    #[test]
    fn running_or_busy_worker_is_work() {
        for state in ["running", "busy"] {
            let status = Status {
                workers: vec![worker(state)],
                ..Status::default()
            };
            assert_eq!(mode(&status), Mode::Work);
        }
    }

    #[test]
    fn watch_alone_is_work() {
        let status = Status {
            watches: vec![watch()],
            ..Status::default()
        };
        assert_eq!(mode(&status), Mode::Work);
    }

    #[test]
    fn finished_idle_and_blocked_workers_are_not_work() {
        for state in ["done", "idle", "blocked", "question"] {
            let status = Status {
                workers: vec![worker(state)],
                ..Status::default()
            };
            assert_eq!(mode(&status), Mode::Idle);
        }
    }

    #[test]
    fn nap_beats_inbox_and_work() {
        let status = Status {
            napping: true,
            unread: 3,
            workers: vec![worker("running")],
            watches: vec![watch()],
            ..Status::default()
        };
        assert_eq!(mode(&status), Mode::Nap);
    }

    #[test]
    fn inbox_beats_work() {
        let status = Status {
            unread: 1,
            workers: vec![worker("running")],
            watches: vec![watch()],
            ..Status::default()
        };
        assert_eq!(mode(&status), Mode::Inbox);
    }

    #[test]
    fn labels_are_upper_case_names() {
        assert_eq!(Mode::Nap.label(), "NAP");
        assert_eq!(Mode::Inbox.label(), "INBOX");
        assert_eq!(Mode::Work.label(), "WORK");
        assert_eq!(Mode::Idle.label(), "IDLE");
    }
}
