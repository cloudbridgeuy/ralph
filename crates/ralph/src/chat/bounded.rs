use ralph_core::bounded_command::{poll_verdict, Poll};
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const STEP: Duration = Duration::from_millis(20);

pub(super) fn stdout_within(command: &mut Command, limit: Duration) -> Option<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).ok().map(|_| bytes)
    });
    let started = Instant::now();
    let success = loop {
        let exit = child.try_wait().ok()?.map(|status| status.success());
        match poll_verdict(exit, started.elapsed(), limit) {
            Poll::Wait => thread::sleep(STEP),
            Poll::Done(success) => break success,
            Poll::Expired => {
                child.kill().ok();
                child.wait().ok();
                return None;
            }
        }
    };
    let bytes = reader.join().ok()??;
    success.then(|| String::from_utf8_lossy(&bytes).trim().to_string())
}
