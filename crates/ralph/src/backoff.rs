use crate::stream_processor::extract_response_text;
use crate::subprocess::{StreamingSubprocessResult, SubprocessError};
use ralph_core::chat_memory::tag_failure_kind;
use ralph_core::retry::{classify, retry_delay, FailureInput, FailureKind, OUTAGE_BACKOFF_SECS};
use ralph_server::warn::warn;
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

/// The finished run behind a failed call, and whether it timed out. `None`
/// for a success or an error that has no run (spawn failure, interrupt).
fn failed_run(result: &SubprocessOutcome) -> Option<(bool, &StreamingSubprocessResult)> {
    match result {
        Ok(done) if done.exit_code != 0 => Some((false, done)),
        Err(SubprocessError::Timeout { partial_result, .. }) => Some((true, partial_result)),
        _ => None,
    }
}

fn failure_of(result: &SubprocessOutcome) -> Option<FailureText> {
    failed_run(result).map(|(timed_out, done)| FailureText {
        timed_out,
        exit_code: done.exit_code,
        text: failure_text(done),
    })
}

pub fn attempt_from(result: SubprocessOutcome) -> Attempt<SubprocessOutcome> {
    match failure_of(&result) {
        Some(failure) => Attempt::Failed(result, failure),
        None => Attempt::Done(result),
    }
}

/// How the last attempt of a tag call failed, or `None` if it did not. Unlike
/// the retry loop, it reads only the `API Error` lines of the reply (see
/// `tag_failure_kind`), and only for a failed call.
pub fn final_tag_failure_kind(result: &SubprocessOutcome) -> Option<FailureKind> {
    let (timed_out, done) = failed_run(result)?;
    let response = extract_response_text(&done.stream_result.output_blocks).unwrap_or_default();
    Some(tag_failure_kind(
        timed_out,
        done.exit_code,
        &done.stderr,
        &response,
    ))
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

    fn finished(exit_code: i32, stderr: &str) -> StreamingSubprocessResult {
        StreamingSubprocessResult {
            exit_code,
            stderr: stderr.to_string(),
            stream_result: Default::default(),
            request_at: None,
            cost_before: None,
        }
    }

    fn timed_out() -> SubprocessOutcome {
        Err(SubprocessError::Timeout {
            timeout_secs: 1,
            partial_result: Box::new(finished(137, "")),
        })
    }

    #[test]
    fn retry_loop_and_tag_check_agree_on_what_failed() {
        let cases: Vec<(SubprocessOutcome, Option<FailureKind>)> = vec![
            (Ok(finished(0, "")), None),
            (Ok(finished(1, "")), Some(FailureKind::Refusal)),
            (
                Ok(finished(1, "API Error: 529 overloaded")),
                Some(FailureKind::Outage),
            ),
            (timed_out(), Some(FailureKind::Timeout)),
        ];
        for (outcome, kind) in cases {
            assert_eq!(final_tag_failure_kind(&outcome), kind);
            assert_eq!(failure_of(&outcome).is_some(), kind.is_some());
        }
    }

    fn replied(exit_code: i32, response: &str) -> SubprocessOutcome {
        use crate::stream_processor::OutputBlock;
        use ralph_core::chunk::{ChunkType, ParsedChunk};
        let mut done = finished(exit_code, "");
        done.stream_result
            .output_blocks
            .push(OutputBlock::text(ParsedChunk {
                chunk_type: ChunkType::Prose,
                content: response.to_string(),
            }));
        Ok(done)
    }

    #[test]
    fn a_lasting_outage_in_the_reply_of_a_failed_call_is_an_outage() {
        let kind = |code, response| final_tag_failure_kind(&replied(code, response));
        assert_eq!(
            kind(1, "API Error: 529 overloaded"),
            Some(FailureKind::Outage)
        );
        assert_eq!(
            kind(1, "ok\n\n api error: 503 unavailable"),
            Some(FailureKind::Outage)
        );
        assert_eq!(
            kind(1, r#"[{"n":503,"rule":true}]"#),
            Some(FailureKind::Refusal)
        );
        assert_eq!(kind(0, "API Error: 400 bad request"), None);
        assert_eq!(kind(0, "API Error: 529 overloaded"), None);
    }

    #[test]
    fn retry_loop_marks_a_timeout_and_keeps_stderr() {
        let failure = failure_of(&timed_out()).unwrap();
        assert!(failure.timed_out);
        assert_eq!(failure.exit_code, 137);
        let failure = failure_of(&Ok(finished(2, "boom"))).unwrap();
        assert!(!failure.timed_out);
        assert!(failure.text.starts_with("boom"));
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
