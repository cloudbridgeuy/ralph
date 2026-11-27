#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Exited,
    TimedOut,
    BadHello,
    ConnectionLost,
    SessionFailed,
    NavigateFailed,
    Fatal,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Retry {
    Again,
    GiveUp,
}

pub fn retry_decision(kind: FailureKind, attempt: u32, max: u32) -> Retry {
    if kind != FailureKind::Fatal && attempt < max {
        Retry::Again
    } else {
        Retry::GiveUp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transient_failure_retries_until_the_last_attempt() {
        for kind in [
            FailureKind::Exited,
            FailureKind::TimedOut,
            FailureKind::BadHello,
            FailureKind::ConnectionLost,
            FailureKind::SessionFailed,
            FailureKind::NavigateFailed,
        ] {
            assert_eq!(retry_decision(kind, 1, 3), Retry::Again, "{kind:?}");
            assert_eq!(retry_decision(kind, 2, 3), Retry::Again, "{kind:?}");
            assert_eq!(retry_decision(kind, 3, 3), Retry::GiveUp, "{kind:?}");
            assert_eq!(retry_decision(kind, 4, 3), Retry::GiveUp, "{kind:?}");
        }
    }

    #[test]
    fn a_fatal_failure_never_retries() {
        assert_eq!(retry_decision(FailureKind::Fatal, 1, 3), Retry::GiveUp);
    }

    #[test]
    fn a_single_attempt_budget_never_retries() {
        assert_eq!(retry_decision(FailureKind::Exited, 1, 1), Retry::GiveUp);
    }
}
