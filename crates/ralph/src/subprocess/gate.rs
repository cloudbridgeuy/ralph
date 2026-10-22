use std::io::{self, Write};
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Chunk {
    stream: Stream,
    text: String,
}

#[derive(Debug, Default)]
pub(super) struct Held {
    chunks: Vec<Chunk>,
}

impl Held {
    pub(super) fn push(&mut self, stream: Stream, text: &str) {
        self.chunks.push(Chunk {
            stream,
            text: text.to_string(),
        });
    }

    pub(super) fn take(&mut self) -> Vec<Chunk> {
        std::mem::take(&mut self.chunks)
    }
}

#[derive(Debug, Default)]
struct GateState {
    paused: bool,
    settled: bool,
    held: Held,
}

#[derive(Debug, Default)]
pub struct OutputGate {
    state: Mutex<GateState>,
}

pub(super) fn write_direct(stream: Stream, text: &str) {
    match stream {
        Stream::Stdout => {
            let mut out = io::stdout();
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
        Stream::Stderr => {
            let mut err = io::stderr();
            let _ = err.write_all(text.as_bytes());
            let _ = err.flush();
        }
    }
}

fn emit(chunk: &Chunk) {
    write_direct(chunk.stream, &chunk.text);
}

impl OutputGate {
    pub fn new() -> OutputGate {
        OutputGate::default()
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn is_paused(&self) -> bool {
        self.lock().paused
    }

    pub fn pause(&self) {
        let mut state = self.lock();
        state.paused = true;
        state.settled = false;
    }

    pub fn is_settled(&self) -> bool {
        self.lock().settled
    }

    pub(super) fn settle(&self) {
        self.lock().settled = true;
    }

    pub fn resume(&self) {
        let mut state = self.lock();
        state.held.take().iter().for_each(emit);
        state.paused = false;
        state.settled = false;
    }

    pub fn write_out(&self, text: &str) {
        self.write(Stream::Stdout, text);
    }

    pub fn write_err(&self, text: &str) {
        self.write(Stream::Stderr, text);
    }

    pub(super) fn write(&self, stream: Stream, text: &str) {
        let mut state = self.lock();
        if state.paused {
            state.held.push(stream, text);
        } else {
            write_direct(stream, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_keeps_order_across_streams() {
        let mut held = Held::default();
        held.push(Stream::Stdout, "a");
        held.push(Stream::Stderr, "b");
        held.push(Stream::Stdout, "c");
        let taken = held.take();
        let seen: Vec<(Stream, &str)> = taken.iter().map(|c| (c.stream, c.text.as_str())).collect();
        assert_eq!(
            seen,
            vec![
                (Stream::Stdout, "a"),
                (Stream::Stderr, "b"),
                (Stream::Stdout, "c")
            ]
        );
    }

    #[test]
    fn paused_write_out_and_write_err_are_held_in_order() {
        let gate = OutputGate::new();
        gate.pause();
        gate.write_out("rule");
        gate.write_err("warning");
        let held = gate.lock().held.take();
        let seen: Vec<(Stream, &str)> = held.iter().map(|c| (c.stream, c.text.as_str())).collect();
        assert_eq!(
            seen,
            vec![(Stream::Stdout, "rule"), (Stream::Stderr, "warning")]
        );
    }

    #[test]
    fn take_empties_the_buffer() {
        let mut held = Held::default();
        held.push(Stream::Stdout, "a");
        assert_eq!(held.take().len(), 1);
        assert!(held.take().is_empty());
    }

    #[test]
    fn writes_while_running_are_not_held() {
        let gate = OutputGate::new();
        gate.write(Stream::Stdout, "");
        assert!(gate.lock().held.take().is_empty());
    }

    #[test]
    fn pause_unsettles_and_settle_marks_until_resume() {
        let gate = OutputGate::new();
        gate.pause();
        assert!(!gate.is_settled());
        gate.settle();
        assert!(gate.is_settled());
        gate.resume();
        assert!(!gate.is_settled());
    }

    #[test]
    fn writes_while_paused_are_held_until_resume() {
        let gate = OutputGate::new();
        gate.pause();
        assert!(gate.is_paused());
        gate.write(Stream::Stdout, "");
        gate.write(Stream::Stderr, "");
        assert_eq!(gate.lock().held.chunks.len(), 2);
        gate.resume();
        assert!(!gate.is_paused());
        assert!(gate.lock().held.take().is_empty());
    }
}
