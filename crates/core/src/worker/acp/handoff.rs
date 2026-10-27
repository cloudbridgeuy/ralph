use std::time::Duration;

pub const HANDOFF_TIMEOUT: Duration = Duration::from_secs(30);
pub const HANDOFF_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handoff {
    Pending,
    Done,
    Failed(String),
    TimedOut,
}

pub fn spawn_handoff(ready: bool, error: Option<&str>, elapsed: Duration) -> Handoff {
    match (error, ready) {
        (Some(error), _) => Handoff::Failed(error.trim().to_string()),
        (None, true) => Handoff::Done,
        (None, false) if elapsed >= HANDOFF_TIMEOUT => Handoff::TimedOut,
        (None, false) => Handoff::Pending,
    }
}

pub fn with_start_warning(started: String, warning: Option<&str>) -> String {
    match warning.map(str::trim).filter(|w| !w.is_empty()) {
        Some(warning) => format!("{started} {warning}"),
        None => started,
    }
}

pub fn close_handoff(owner_exists: bool, elapsed: Duration) -> Handoff {
    if !owner_exists {
        Handoff::Done
    } else if elapsed >= HANDOFF_TIMEOUT {
        Handoff::TimedOut
    } else {
        Handoff::Pending
    }
}

pub fn request_stale(requested_at: u64, now: u64) -> bool {
    now.saturating_sub(requested_at) >= HANDOFF_TIMEOUT.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_handoff_prefers_error_then_ready_then_timeout() {
        let early = Duration::from_secs(1);
        assert_eq!(spawn_handoff(false, None, early), Handoff::Pending);
        assert_eq!(spawn_handoff(true, None, early), Handoff::Done);
        assert_eq!(
            spawn_handoff(true, Some("bad\n"), early),
            Handoff::Failed("bad".into())
        );
        assert_eq!(
            spawn_handoff(false, None, HANDOFF_TIMEOUT),
            Handoff::TimedOut
        );
        assert_eq!(spawn_handoff(true, None, HANDOFF_TIMEOUT), Handoff::Done);
    }

    #[test]
    fn start_warning_is_appended_only_when_it_has_text() {
        assert_eq!(with_start_warning("ok.".into(), None), "ok.");
        assert_eq!(with_start_warning("ok.".into(), Some(" \n")), "ok.");
        assert_eq!(with_start_warning("ok.".into(), Some("old\n")), "ok. old");
    }

    #[test]
    fn close_handoff_is_done_once_the_owner_file_is_gone() {
        let early = Duration::from_secs(1);
        assert_eq!(close_handoff(true, early), Handoff::Pending);
        assert_eq!(close_handoff(false, early), Handoff::Done);
        assert_eq!(close_handoff(true, HANDOFF_TIMEOUT), Handoff::TimedOut);
    }

    #[test]
    fn requests_older_than_the_timeout_are_stale() {
        assert!(!request_stale(100, 129));
        assert!(request_stale(100, 130));
        assert!(!request_stale(100, 50));
    }
}
