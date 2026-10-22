use std::time::Duration;

/// How long calls already started may finish after SIGINT, SIGTERM, or SIGHUP.
pub const SHUTDOWN_DRAIN: Duration = Duration::from_secs(2);

pub const INTERRUPTED: &str = "interrupted";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainAction {
    Wait,
    Finish,
}

/// Keep waiting while a call is still open and the drain has not run out.
pub fn drain_action(elapsed: Duration, limit: Duration, open: usize) -> DrainAction {
    if open == 0 || elapsed >= limit {
        DrainAction::Finish
    } else {
        DrainAction::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_only_while_a_call_is_open_inside_the_drain() {
        let limit = SHUTDOWN_DRAIN;
        assert_eq!(
            drain_action(Duration::from_millis(0), limit, 1),
            DrainAction::Wait
        );
        assert_eq!(
            drain_action(limit - Duration::from_nanos(1), limit, 2),
            DrainAction::Wait
        );
        assert_eq!(drain_action(limit, limit, 1), DrainAction::Finish);
        assert_eq!(
            drain_action(limit + Duration::from_secs(5), limit, 3),
            DrainAction::Finish
        );
        assert_eq!(
            drain_action(Duration::from_millis(0), limit, 0),
            DrainAction::Finish
        );
    }
}
