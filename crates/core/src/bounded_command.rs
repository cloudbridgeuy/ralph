use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    Wait,
    Done(bool),
    Expired,
}

pub fn poll_verdict(exit: Option<bool>, elapsed: Duration, limit: Duration) -> Poll {
    match exit {
        Some(success) => Poll::Done(success),
        None if elapsed >= limit => Poll::Expired,
        None => Poll::Wait,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: Duration = Duration::from_secs(3);

    #[test]
    fn a_running_command_inside_the_limit_waits() {
        assert_eq!(poll_verdict(None, Duration::ZERO, LIMIT), Poll::Wait);
        assert_eq!(
            poll_verdict(None, Duration::from_millis(2999), LIMIT),
            Poll::Wait
        );
    }

    #[test]
    fn a_running_command_at_the_limit_expires() {
        assert_eq!(poll_verdict(None, LIMIT, LIMIT), Poll::Expired);
        assert_eq!(
            poll_verdict(None, Duration::from_secs(60), LIMIT),
            Poll::Expired
        );
    }

    #[test]
    fn an_exited_command_is_done_even_after_the_limit() {
        for elapsed in [Duration::ZERO, LIMIT, Duration::from_secs(60)] {
            assert_eq!(poll_verdict(Some(true), elapsed, LIMIT), Poll::Done(true));
            assert_eq!(poll_verdict(Some(false), elapsed, LIMIT), Poll::Done(false));
        }
    }
}
