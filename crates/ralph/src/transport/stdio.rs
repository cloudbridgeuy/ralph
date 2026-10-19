use crate::cursor_owner::{signal_group, TERM_GRACE, TERM_POLL};
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) fn spawn_piped(
    mut command: Command,
) -> Result<(Child, ChildStdin, ChildStdout, ChildStderr), io::Error> {
    let mut child = command
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    match child
        .stdin
        .take()
        .zip(child.stdout.take())
        .zip(child.stderr.take())
    {
        Some(((stdin, stdout), stderr)) => Ok((child, stdin, stdout, stderr)),
        None => {
            stop_child(&mut child);
            Err(io::Error::other("child has no stdio pipes"))
        }
    }
}

pub(crate) fn stop_groups(groups: &[u32], grace: Duration, mut alive: impl FnMut() -> bool) {
    for group in groups {
        signal_group(*group, "TERM");
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline && alive() {
        thread::sleep(TERM_POLL);
    }
    for group in groups {
        signal_group(*group, "KILL");
    }
}

pub(crate) fn stop_group(group: u32, alive: impl FnMut() -> bool) {
    stop_groups(&[group], TERM_GRACE, alive);
}

pub(crate) fn stop_child(child: &mut Child) {
    let group = child.id();
    stop_group(group, || matches!(child.try_wait(), Ok(None)));
    child.wait().ok();
}
