use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    Wait,
    Done(bool),
    Expired,
    Kill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Running,
    Exited(bool),
    Errored,
}

pub fn poll_verdict(probe: Probe, elapsed: Duration, limit: Duration) -> Poll {
    match probe {
        Probe::Exited(success) => Poll::Done(success),
        Probe::Errored => Poll::Kill,
        Probe::Running if elapsed >= limit => Poll::Expired,
        Probe::Running => Poll::Wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: Duration = Duration::from_secs(3);

    #[test]
    fn a_running_command_inside_the_limit_waits() {
        assert_eq!(
            poll_verdict(Probe::Running, Duration::ZERO, LIMIT),
            Poll::Wait
        );
        assert_eq!(
            poll_verdict(Probe::Running, Duration::from_millis(2999), LIMIT),
            Poll::Wait
        );
    }

    #[test]
    fn a_running_command_at_the_limit_expires() {
        assert_eq!(poll_verdict(Probe::Running, LIMIT, LIMIT), Poll::Expired);
        assert_eq!(
            poll_verdict(Probe::Running, Duration::from_secs(60), LIMIT),
            Poll::Expired
        );
    }

    #[test]
    fn an_exited_command_is_done_even_after_the_limit() {
        for elapsed in [Duration::ZERO, LIMIT, Duration::from_secs(60)] {
            assert_eq!(
                poll_verdict(Probe::Exited(true), elapsed, LIMIT),
                Poll::Done(true)
            );
            assert_eq!(
                poll_verdict(Probe::Exited(false), elapsed, LIMIT),
                Poll::Done(false)
            );
        }
    }

    #[test]
    fn a_probe_error_kills_the_command_at_once_or_late() {
        for elapsed in [Duration::ZERO, LIMIT, Duration::from_secs(60)] {
            assert_eq!(
                poll_verdict(Probe::Errored, elapsed, LIMIT),
                Poll::Kill,
                "{elapsed:?}"
            );
        }
    }
}
