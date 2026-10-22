use crate::chat::editor::{build_queue_editor, CLOSE_QUEUE};
use crate::chat::monitor::Monitor;
use crate::chat::prompt::{retry_interrupted, QueuePrompt};
use crate::chat::queue::{
    has_queue_key, on_exit, typed_after_queue_key, Action, EditorExit, Queue,
};
use crate::chat::statusbar::StatusBar;
use crate::chat::terminal::PartialMode;
use crate::subprocess::OutputGate;
use ralph_core::chat_edit_mode::EditMode as EditModeConfig;
use reedline::{EditCommand, Reedline, Signal};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const POLL_MS: libc::c_int = 50;
const SETTLE_LIMIT: Duration = Duration::from_millis(300);
const SETTLE_STEP: Duration = Duration::from_millis(10);
const READ_CHUNK: usize = 256;

pub(super) struct Keys<'a> {
    pub(super) gate: &'a OutputGate,
    pub(super) cancel: &'a AtomicBool,
    pub(super) monitor: &'a Monitor,
    pub(super) bar: &'a StatusBar,
    pub(super) edit_mode: EditModeConfig,
}

struct Finished<'a>(&'a AtomicBool);

impl Drop for Finished<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

pub(super) fn exit_for(signal: Signal) -> Option<EditorExit> {
    match signal {
        Signal::Success(line) => Some(EditorExit::Submitted(line)),
        Signal::CtrlC => Some(EditorExit::Cancelled),
        Signal::CtrlD => Some(EditorExit::Closed),
        Signal::HostCommand(command) if command == CLOSE_QUEUE => Some(EditorExit::Closed),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Input {
    Ready,
    Idle,
    Closed,
}

fn classify_poll(result: libc::c_int, revents: libc::c_short) -> Input {
    if result <= 0 {
        Input::Idle
    } else if revents & libc::POLLIN != 0 {
        Input::Ready
    } else if revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
        Input::Closed
    } else {
        Input::Idle
    }
}

fn poll_input() -> Input {
    let mut input = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut input, 1, POLL_MS) };
    classify_poll(result, input.revents)
}

fn read_input() -> Vec<u8> {
    let mut buffer = [0u8; READ_CHUNK];
    let count = unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
    usize::try_from(count)
        .map(|count| buffer[..count].to_vec())
        .unwrap_or_default()
}

fn wait_settled(gate: &OutputGate) {
    let started = Instant::now();
    while !gate.is_settled() && started.elapsed() < SETTLE_LIMIT {
        thread::sleep(SETTLE_STEP);
    }
}

fn read_exit(editor: &mut Reedline, prompt: &QueuePrompt) -> EditorExit {
    loop {
        match retry_interrupted(|| editor.read_line(prompt)) {
            Ok(signal) => {
                if let Some(exit) = exit_for(signal) {
                    return exit;
                }
            }
            Err(_) => return EditorExit::Closed,
        }
    }
}

impl Keys<'_> {
    pub(super) fn during<R>(&self, run: impl FnOnce() -> R) -> (R, Queue) {
        let Some(_mode) = std::io::stdin()
            .is_terminal()
            .then(PartialMode::enter)
            .flatten()
        else {
            return (run(), Queue::default());
        };
        let done = AtomicBool::new(false);
        let (result, queue) = thread::scope(|scope| {
            let finished = Finished(&done);
            let listener = scope.spawn(|| self.listen(&done));
            let result = run();
            drop(finished);
            (result, listener.join().unwrap_or_default())
        });
        self.cancel.store(false, Ordering::SeqCst);
        self.gate.resume();
        (result, queue)
    }

    fn listen(&self, done: &AtomicBool) -> Queue {
        let mut queue = Queue::default();
        while !done.load(Ordering::SeqCst) {
            match poll_input() {
                Input::Idle => continue,
                Input::Closed => break,
                Input::Ready => {}
            }
            let bytes = read_input();
            if bytes.is_empty() {
                thread::sleep(Duration::from_millis(POLL_MS as u64));
            } else if has_queue_key(&bytes) && !done.load(Ordering::SeqCst) {
                queue = self.edit(queue, &typed_after_queue_key(&bytes));
            }
        }
        queue
    }

    fn edit(&self, queue: Queue, seed: &str) -> Queue {
        self.gate.pause();
        wait_settled(self.gate);
        let mut editor = build_queue_editor(self.edit_mode);
        if !seed.is_empty() {
            editor.run_edit_commands(&[EditCommand::InsertString(seed.to_string())]);
        }
        let prompt = QueuePrompt {
            monitor: self.monitor,
            bar: self.bar,
            queued: queue.entries.len(),
        };
        let exit = read_exit(&mut editor, &prompt);
        let (queue, action) = on_exit(queue, exit);
        self.gate.resume();
        if action == Action::Cancel {
            self.cancel.store(true, Ordering::SeqCst);
        }
        queue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_with_data_is_ready_even_when_hung_up() {
        assert_eq!(classify_poll(1, libc::POLLIN), Input::Ready);
        assert_eq!(classify_poll(1, libc::POLLIN | libc::POLLHUP), Input::Ready);
    }

    #[test]
    fn poll_hangup_error_or_invalid_is_closed() {
        assert_eq!(classify_poll(1, libc::POLLHUP), Input::Closed);
        assert_eq!(classify_poll(1, libc::POLLERR), Input::Closed);
        assert_eq!(classify_poll(1, libc::POLLNVAL), Input::Closed);
    }

    #[test]
    fn poll_timeout_or_failure_is_idle() {
        assert_eq!(classify_poll(0, 0), Input::Idle);
        assert_eq!(classify_poll(-1, 0), Input::Idle);
    }

    #[test]
    fn enter_submits_the_line() {
        assert_eq!(
            exit_for(Signal::Success("hi".into())),
            Some(EditorExit::Submitted("hi".into()))
        );
    }

    #[test]
    fn ctrl_c_cancels() {
        assert_eq!(exit_for(Signal::CtrlC), Some(EditorExit::Cancelled));
    }

    #[test]
    fn ctrl_d_and_ctrl_q_close() {
        assert_eq!(exit_for(Signal::CtrlD), Some(EditorExit::Closed));
        assert_eq!(
            exit_for(Signal::HostCommand(CLOSE_QUEUE.into())),
            Some(EditorExit::Closed)
        );
    }

    #[test]
    fn other_host_commands_keep_the_editor_open() {
        assert_eq!(exit_for(Signal::HostCommand("other".into())), None);
        assert_eq!(exit_for(Signal::ExternalBreak(String::new())), None);
    }
}
