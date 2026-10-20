use super::kill_process_group;
use super::spinner::{spawn_piped, Pump, SpinnerSubprocessConfig};
use super::timeout::try_wait_child;
use super::types::{
    StreamingSubprocessResult, SubprocessError, EXIT_CODE_INTERRUPTED, EXIT_CODE_KILLED,
};
use crate::signal;
use ralph_core::stream::{interrupt_line, user_line};
use std::io::{self, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

const INTERRUPT_GRACE: Duration = Duration::from_secs(3);
const EXIT_GRACE: Duration = Duration::from_secs(3);
const INTERRUPT_REQUEST_ID: &str = "int-1";

pub struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    config: SpinnerSubprocessConfig,
}

impl Session {
    pub fn spawn(config: &SpinnerSubprocessConfig) -> Result<Session, SubprocessError> {
        let mut child = spawn_piped(&config.command, Stdio::piped())?;
        let stdin = child.stdin.take();
        Ok(Session {
            child,
            stdin,
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

    fn close(&mut self) -> i32 {
        self.stdin.take();
        let deadline = Instant::now() + EXIT_GRACE;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.code().unwrap_or(EXIT_CODE_KILLED),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                _ => {
                    kill_process_group(&mut self.child);
                    return EXIT_CODE_KILLED;
                }
            }
        }
    }

    pub fn turn(&mut self, prompt: &str) -> Result<StreamingSubprocessResult, SubprocessError> {
        let mut pump = Pump::start(&mut self.child, &self.config)?;
        self.send(&user_line(prompt))?;
        let mut interrupt: Option<(Instant, bool)> = None;
        let outcome = loop {
            if let Some(status) = try_wait_child(&mut self.child)? {
                let exit_code = status.code().ok_or(SubprocessError::Signaled)?;
                break pump.finish(exit_code, true);
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
                kill_process_group(&mut self.child);
                break pump.finish(EXIT_CODE_KILLED, false);
            }
            let paused = self
                .config
                .pause
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst));
            if pump.step(paused)? {
                let exit_code = self.close();
                break pump.finish(exit_code, true);
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
