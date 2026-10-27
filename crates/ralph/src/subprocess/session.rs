use super::spinner::{spawn_piped, Display, Pump, Readers, SpinnerSubprocessConfig, Step};
use super::types::{
    StreamingSubprocessResult, SubprocessError, EXIT_CODE_INTERRUPTED, EXIT_CODE_KILLED,
};
use super::{exited_unreaped, signal_group};
use crate::signal;
use ralph_core::stream::{interrupt_line, result_is_error, user_line};
use std::io::{self, Write};
use std::process::{Child, ChildStdin, ExitStatus, Stdio};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

const INTERRUPT_GRACE: Duration = Duration::from_secs(3);
const EXIT_GRACE: Duration = Duration::from_secs(3);
const INTERRUPT_REQUEST_ID: &str = "int-1";

fn leftover_is_stale(lines: &[String]) -> bool {
    lines.iter().any(|line| result_is_error(line).is_some())
}

fn drain_after_exit(
    deadline: Instant,
    mut step: impl FnMut() -> Result<Step, SubprocessError>,
) -> Result<Option<bool>, SubprocessError> {
    while Instant::now() < deadline {
        match step()? {
            Step::Pending => {}
            Step::Result { is_error } => return Ok(Some(is_error)),
            Step::Closed => return Ok(None),
        }
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildState {
    Running,
    ExitedUnreaped,
    Reaped,
}

/// After the child is reaped its pid may belong to another process, so the
/// group must not be signalled again.
fn group_signal_safe(state: ChildState) -> bool {
    state != ChildState::Reaped
}

pub struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    readers: Readers,
    alive: bool,
    reaped: bool,
    config: SpinnerSubprocessConfig,
}

impl Session {
    pub fn spawn(config: &SpinnerSubprocessConfig) -> Result<Session, SubprocessError> {
        let mut child = spawn_piped(&config.command, Stdio::piped())?;
        let stdin = child.stdin.take();
        let readers = Readers::start(&mut child)?;
        Ok(Session {
            child,
            stdin,
            readers,
            alive: true,
            reaped: false,
            config: config.clone(),
        })
    }

    fn send(&mut self, line: &str) -> io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?;
        writeln!(stdin, "{line}")?;
        stdin.flush()
    }

    fn state(&self) -> ChildState {
        if self.reaped {
            ChildState::Reaped
        } else if exited_unreaped(&self.child) {
            ChildState::ExitedUnreaped
        } else {
            ChildState::Running
        }
    }

    /// Kills the process group while the child still holds its pid, then
    /// reaps the child. Later calls only return the cached status.
    fn reap(&mut self) -> Option<ExitStatus> {
        if group_signal_safe(self.state()) {
            signal_group(&self.child);
        }
        self.reaped = true;
        self.child.wait().ok()
    }

    fn close(&mut self) -> i32 {
        self.stdin.take();
        let deadline = Instant::now() + EXIT_GRACE;
        while self.state() == ChildState::Running && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        self.reap()
            .and_then(|status| status.code())
            .unwrap_or(EXIT_CODE_KILLED)
    }

    pub fn is_alive(&mut self) -> bool {
        self.alive = self.alive && self.state() == ChildState::Running;
        self.alive
    }

    pub fn turn(&mut self, prompt: &str) -> Result<StreamingSubprocessResult, SubprocessError> {
        let result = self.drive(prompt, Display::Shown, None);
        if matches!(&result, Err(e) if !matches!(e, SubprocessError::Interrupted { .. })) {
            self.alive = false;
        }
        result
    }

    pub fn ping(&mut self, prompt: &str, timeout: Duration) -> Result<(), SubprocessError> {
        let outcome = match self.drive(prompt, Display::Silent, Some(timeout)) {
            Ok(result) if result.exit_code == 0 => return Ok(()),
            Ok(_) => SubprocessError::Stale,
            Err(e) => e,
        };
        self.reap();
        self.alive = false;
        Err(outcome)
    }

    fn drive(
        &mut self,
        prompt: &str,
        display: Display,
        timeout: Option<Duration>,
    ) -> Result<StreamingSubprocessResult, SubprocessError> {
        if leftover_is_stale(&self.readers.take_queued()) {
            return Err(SubprocessError::LeftoverResult);
        }
        let mut pump = Pump::start(&self.config, display)?;
        let deadline = timeout.map(|limit| Instant::now() + limit);
        self.send(&user_line(prompt))
            .map_err(|_| SubprocessError::Stale)?;
        let mut interrupt: Option<(Instant, bool)> = None;
        let outcome = loop {
            if self.state() != ChildState::Running {
                self.alive = false;
                let status = self.reap();
                let drained = drain_after_exit(Instant::now() + EXIT_GRACE, || {
                    pump.step(&self.readers, true)
                })?;
                let exit_code = match drained {
                    Some(is_error) => i32::from(is_error),
                    None => status
                        .and_then(|status| status.code())
                        .ok_or(SubprocessError::Signaled)?,
                };
                break pump.finish(&self.readers, exit_code, true);
            }
            let cancelled = self
                .config
                .cancel
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst));
            if interrupt.is_none() && (cancelled || signal::is_interrupted()) {
                let _ = self.send(&interrupt_line(INTERRUPT_REQUEST_ID));
                interrupt = Some((Instant::now(), cancelled));
            }
            if interrupt.is_some_and(|(sent, _)| sent.elapsed() >= INTERRUPT_GRACE) {
                self.reap();
                self.alive = false;
                break pump.finish(&self.readers, EXIT_CODE_KILLED, false);
            }
            if let (Some(limit), Some(at)) = (timeout, deadline) {
                if Instant::now() >= at {
                    self.reap();
                    self.alive = false;
                    return Err(SubprocessError::Timeout {
                        timeout_secs: limit.as_secs(),
                        partial_result: Box::new(pump.finish(
                            &self.readers,
                            EXIT_CODE_KILLED,
                            false,
                        )),
                    });
                }
            }
            match pump.step(&self.readers, pump.gate_paused())? {
                Step::Pending => {}
                Step::Result { is_error } => {
                    break pump.finish(&self.readers, i32::from(is_error), true);
                }
                Step::Closed => {
                    self.alive = false;
                    let exit_code = self.close();
                    break pump.finish(&self.readers, exit_code, true);
                }
            }
        };
        match interrupt {
            None => Ok(outcome),
            Some((_, cancelled)) => {
                let partial = StreamingSubprocessResult {
                    exit_code: EXIT_CODE_INTERRUPTED,
                    ..outcome
                };
                if cancelled {
                    Ok(partial)
                } else {
                    Err(SubprocessError::Interrupted {
                        partial_result: Box::new(partial),
                    })
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(raw: &[&str]) -> Vec<String> {
        raw.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn running_child_group_is_safe_to_signal() {
        assert!(group_signal_safe(ChildState::Running));
    }

    #[test]
    fn exited_unreaped_child_group_is_safe_to_signal() {
        assert!(group_signal_safe(ChildState::ExitedUnreaped));
    }

    #[test]
    fn reaped_child_group_is_not_signalled_again() {
        assert!(!group_signal_safe(ChildState::Reaped));
    }

    #[test]
    fn no_leftover_is_not_stale() {
        assert!(!leftover_is_stale(&[]));
    }

    #[test]
    fn leftover_system_and_assistant_lines_are_not_stale() {
        assert!(!leftover_is_stale(&lines(&[
            r#"{"type":"system","subtype":"status"}"#,
            r#"{"type":"assistant","message":{"content":[]}}"#,
        ])));
    }

    #[test]
    fn leftover_result_is_stale() {
        assert!(leftover_is_stale(&lines(&[
            r#"{"type":"system","subtype":"status"}"#,
            r#"{"type":"result","subtype":"success","is_error":false}"#,
        ])));
    }

    #[test]
    fn leftover_nested_result_text_is_not_stale() {
        assert!(!leftover_is_stale(&lines(&[
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","input":{"type":"result"}}]}}"#,
        ])));
    }

    fn scripted(
        steps: Vec<Result<Step, SubprocessError>>,
    ) -> impl FnMut() -> Result<Step, SubprocessError> {
        let mut queue = std::collections::VecDeque::from(steps);
        move || queue.pop_front().unwrap_or(Ok(Step::Closed))
    }

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(60)
    }

    #[test]
    fn drain_returns_result_after_pending() {
        let step = scripted(vec![
            Ok(Step::Pending),
            Ok(Step::Pending),
            Ok(Step::Result { is_error: true }),
        ]);
        assert_eq!(drain_after_exit(far(), step).unwrap(), Some(true));
    }

    #[test]
    fn drain_returns_none_when_closed() {
        let step = scripted(vec![Ok(Step::Pending), Ok(Step::Closed)]);
        assert_eq!(drain_after_exit(far(), step).unwrap(), None);
    }

    #[test]
    fn drain_past_deadline_never_steps() {
        let mut calls = 0;
        let outcome = drain_after_exit(Instant::now(), || {
            calls += 1;
            Ok(Step::Pending)
        });
        assert_eq!(outcome.unwrap(), None);
        assert_eq!(calls, 0);
    }

    #[test]
    fn drain_propagates_errors() {
        let step = scripted(vec![Err(SubprocessError::OutputCaptureFailed(
            "boom".to_string(),
        ))]);
        assert!(matches!(
            drain_after_exit(far(), step),
            Err(SubprocessError::OutputCaptureFailed(_))
        ));
    }
}
