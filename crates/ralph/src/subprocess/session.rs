use super::kill_process_group;
use super::spinner::{spawn_piped, Pump, Readers, SpinnerSubprocessConfig, Step};
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
    readers: Readers,
    alive: bool,
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
        let exit_code = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status.code().unwrap_or(EXIT_CODE_KILLED),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                _ => break EXIT_CODE_KILLED,
            }
        };
        kill_process_group(&mut self.child);
        exit_code
    }

    pub fn is_alive(&self) -> bool {
        self.alive
    }

    pub fn turn(&mut self, prompt: &str) -> Result<StreamingSubprocessResult, SubprocessError> {
        let result = self.drive(prompt);
        if matches!(&result, Err(e) if !matches!(e, SubprocessError::Interrupted { .. })) {
            self.alive = false;
        }
        result
    }

    fn drive(&mut self, prompt: &str) -> Result<StreamingSubprocessResult, SubprocessError> {
        let mut pump = Pump::start(&self.config)?;
        self.send(&user_line(prompt))?;
        let mut interrupt: Option<(Instant, bool)> = None;
        let outcome = loop {
            if let Some(status) = try_wait_child(&mut self.child)? {
                self.alive = false;
                let exit_code = status.code().ok_or(SubprocessError::Signaled)?;
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
                kill_process_group(&mut self.child);
                self.alive = false;
                break pump.finish(&self.readers, EXIT_CODE_KILLED, false);
            }
            let paused = self
                .config
                .pause
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst));
            match pump.step(&self.readers, paused)? {
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
