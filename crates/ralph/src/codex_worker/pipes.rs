use super::Stdin;
use crate::acp_log::{CappedLog, LoggedRead, LoggedWrite};
use std::io::{BufRead, BufReader};
use std::process::{ChildStdin, ChildStdout};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

pub(super) fn logged_stdin(stdin: ChildStdin, log: &Arc<CappedLog>) -> Stdin {
    Arc::new(Mutex::new(Some(LoggedWrite {
        inner: stdin,
        log: Arc::clone(log),
        tag: "in",
    })))
}

pub(super) fn pipe_lines(
    stdout: ChildStdout,
    log: &Arc<CappedLog>,
) -> (mpsc::Receiver<String>, mpsc::Receiver<()>) {
    let (lines_tx, lines) = mpsc::channel();
    let (finished_tx, finished) = mpsc::channel();
    let reader_log = Arc::clone(log);
    thread::spawn(move || {
        let reader = BufReader::new(LoggedRead {
            inner: stdout,
            log: reader_log,
            tag: "out",
        });
        for line in reader.lines().map_while(Result::ok) {
            if lines_tx.send(line).is_err() {
                break;
            }
        }
        finished_tx.send(()).ok();
    });
    (lines, finished)
}
