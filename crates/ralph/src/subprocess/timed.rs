use super::kill_process_group;
use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TimedOutput {
    Finished { success: bool, stdout: Vec<u8> },
    TimedOut,
    Failed(String),
}

/// Runs the command in its own process group with null stdin and stderr and
/// kills the group when it does not finish within the limit.
pub(crate) fn timed_output(command: &mut Command, limit: Duration) -> TimedOutput {
    let deadline = Instant::now() + limit;
    let spawned = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return TimedOutput::Failed(error.to_string()),
    };
    let Some(mut stdout) = child.stdout.take() else {
        kill_process_group(&mut child);
        return TimedOutput::Failed("no stdout".to_string());
    };
    let (sender, receiver) = mpsc::channel();
    let reader = thread::Builder::new().spawn(move || {
        let mut buffer = Vec::new();
        let _ = sender.send(stdout.read_to_end(&mut buffer).map(|_| buffer));
    });
    if let Err(error) = reader {
        kill_process_group(&mut child);
        return TimedOutput::Failed(error.to_string());
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                kill_process_group(&mut child);
                return TimedOutput::TimedOut;
            }
            Err(error) => {
                kill_process_group(&mut child);
                return TimedOutput::Failed(error.to_string());
            }
        }
    };
    received(
        status.success(),
        receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())),
    )
}

fn received(success: bool, read: Result<io::Result<Vec<u8>>, RecvTimeoutError>) -> TimedOutput {
    match read {
        Ok(Ok(stdout)) => TimedOutput::Finished { success, stdout },
        Ok(Err(error)) => TimedOutput::Failed(error.to_string()),
        Err(RecvTimeoutError::Timeout) => TimedOutput::TimedOut,
        Err(RecvTimeoutError::Disconnected) => {
            TimedOutput::Failed("the output reader stopped before it sent the output".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_that_dies_is_a_failure_and_only_a_late_reader_times_out() {
        assert_eq!(
            received(true, Ok(Ok(b"1.0".to_vec()))),
            TimedOutput::Finished {
                success: true,
                stdout: b"1.0".to_vec()
            }
        );
        assert_eq!(
            received(true, Err(RecvTimeoutError::Timeout)),
            TimedOutput::TimedOut
        );
        assert!(matches!(
            received(true, Err(RecvTimeoutError::Disconnected)),
            TimedOutput::Failed(_)
        ));
    }
}
