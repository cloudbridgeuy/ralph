use crate::stream_processor::extract_response_text;
use crate::subprocess::{StreamingSubprocessResult, SubprocessError};
use crate::warn::warn;
use ralph_core::retry::{classify, retry_delay, FailureInput, OUTAGE_BACKOFF_SECS};
use std::time::{Duration, Instant};

const WAIT_SLICE: Duration = Duration::from_millis(100);

pub enum Attempt<T> {
    Done(T),
    Failed(T, FailureText),
}

pub struct FailureText {
    pub timed_out: bool,
    pub exit_code: i32,
    pub text: String,
}

pub type SubprocessOutcome = Result<StreamingSubprocessResult, SubprocessError>;

pub fn attempt_from(result: SubprocessOutcome) -> Attempt<SubprocessOutcome> {
    let failure = match &result {
        Ok(done) if done.exit_code != 0 => Some(FailureText {
            timed_out: false,
            exit_code: done.exit_code,
            text: failure_text(done),
        }),
        Err(SubprocessError::Timeout { partial_result, .. }) => Some(FailureText {
            timed_out: true,
            exit_code: partial_result.exit_code,
            text: failure_text(partial_result),
        }),
        _ => None,
    };
    match failure {
        Some(failure) => Attempt::Failed(result, failure),
        None => Attempt::Done(result),
    }
}

fn failure_text(result: &StreamingSubprocessResult) -> String {
    let response = extract_response_text(&result.stream_result.output_blocks).unwrap_or_default();
    format!("{}\n{response}", result.stderr)
}

pub fn retry_on_outage<T>(
    what: &str,
    mut attempt: impl FnMut() -> Attempt<T>,
    mut wait: impl FnMut(Duration) -> bool,
) -> T {
    let mut retries = 0;
    loop {
        let (value, failure) = match attempt() {
            Attempt::Done(value) => return value,
            Attempt::Failed(value, failure) => (value, failure),
        };
        let kind = classify(&FailureInput {
            timed_out: failure.timed_out,
            exit_code: failure.exit_code,
            text: &failure.text,
        });
        let Some(delay) = retry_delay(kind, retries) else {
            return value;
        };
        warn(format!(
            "{} hit an outage ({}); retrying in {}s ({}/{})",
            what,
            kind.label(),
            delay.as_secs(),
            retries + 1,
            OUTAGE_BACKOFF_SECS.len()
        ));
        if !wait(delay) {
            return value;
        }
        retries += 1;
    }
}

pub fn wait_unless(stop: impl Fn() -> bool, delay: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < delay {
        if stop() {
            return false;
        }
        std::thread::sleep(WAIT_SLICE.min(delay.saturating_sub(start.elapsed())));
    }
    !stop()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn outage(value: u32) -> Attempt<u32> {
        Attempt::Failed(
            value,
            FailureText {
                timed_out: false,
                exit_code: 1,
                text: "API Error: 529 overloaded".into(),
            },
        )
    }

    fn timeout(value: u32) -> Attempt<u32> {
        Attempt::Failed(
            value,
            FailureText {
                timed_out: true,
                exit_code: -1,
                text: String::new(),
            },
        )
    }

    fn refusal(value: u32) -> Attempt<u32> {
        Attempt::Failed(
            value,
            FailureText {
                timed_out: false,
                exit_code: 1,
                text: "no".into(),
            },
        )
    }

    fn run(script: Vec<Attempt<u32>>, keep_waiting: bool) -> (u32, Vec<Duration>) {
        let script = RefCell::new(script.into_iter());
        let waits = RefCell::new(Vec::new());
        let value = retry_on_outage(
            "test",
            || script.borrow_mut().next().expect("scripted attempt"),
            |delay| {
                waits.borrow_mut().push(delay);
                keep_waiting
            },
        );
        (value, waits.into_inner())
    }

    #[test]
    fn success_runs_once_without_waiting() {
        let (value, waits) = run(vec![Attempt::Done(1)], true);
        assert_eq!((value, waits.len()), (1, 0));
    }

    #[test]
    fn outage_retries_then_succeeds() {
        let (value, waits) = run(vec![outage(1), outage(2), Attempt::Done(3)], true);
        assert_eq!(value, 3);
        assert_eq!(waits, [Duration::from_secs(2), Duration::from_secs(8)]);
    }

    #[test]
    fn outage_stops_after_the_schedule() {
        let (value, waits) = run(vec![outage(1), outage(2), outage(3)], true);
        assert_eq!((value, waits.len()), (3, 2));
    }

    #[test]
    fn refusal_is_not_retried() {
        let (value, waits) = run(vec![refusal(1)], true);
        assert_eq!((value, waits.len()), (1, 0));
    }

    #[test]
    fn timeout_is_not_retried() {
        let (value, waits) = run(vec![timeout(1)], true);
        assert_eq!((value, waits.len()), (1, 0));
    }

    #[test]
    fn stopped_wait_returns_the_failed_value() {
        let (value, waits) = run(vec![outage(1)], false);
        assert_eq!((value, waits.len()), (1, 1));
    }

    #[test]
    fn wait_unless_stops_when_asked() {
        let calls = Cell::new(0);
        let kept = wait_unless(
            || {
                calls.set(calls.get() + 1);
                calls.get() > 1
            },
            Duration::from_secs(5),
        );
        assert!(!kept);
    }

    #[test]
    fn wait_unless_completes_a_short_delay() {
        assert!(wait_unless(|| false, Duration::from_millis(1)));
    }
}
