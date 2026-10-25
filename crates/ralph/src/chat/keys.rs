use crate::chat::editor::{build_queue_editor, BACK_OUT, QUEUE_CTRL_C};
use crate::chat::monitor::Monitor;
use crate::chat::prompt::{retry_interrupted, QueuePrompt};
use crate::chat::queue::{on_exit, Action, EditorExit, Queue};
use crate::chat::route::{run_local, LocalRoute};
use crate::chat::statusbar::StatusBar;
use crate::chat::terminal::{poll_input, read_input, Input, PartialMode, POLL_MS};
use crate::subprocess::OutputGate;
use crate::warn::warn_if_err;
use crate::watches::locked;
use ralph_core::chat_edit_mode::EditMode as EditModeConfig;
use ralph_core::chat_status::{route_input, EmptySubmit, Route};
use ralph_core::chat_typed::{incomplete, queue_ctrl_c, typed, QueueCtrlC, Typed};
use reedline::{EditCommand, Reedline, Signal};
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

const SETTLE_LIMIT: Duration = Duration::from_millis(300);
const SETTLE_STEP: Duration = Duration::from_millis(10);
const JOIN_POLL_MS: libc::c_int = 10;
const JOIN_TRIES: usize = 2;

pub(super) struct Keys<'a> {
    pub(super) gate: &'a OutputGate,
    pub(super) cancel: &'a AtomicBool,
    pub(super) monitor: &'a Monitor,
    pub(super) bar: &'a StatusBar,
    pub(super) edit_mode: EditModeConfig,
    pub(super) local: &'a LocalRoute<'a>,
    pub(super) empty_submit: &'a EmptySubmit,
}

/// The turn's end, as the queue editor sees it: `done` feeds the status bar,
/// and `repaint` wakes the open editor so the bar flips without a key press.
#[derive(Default)]
struct TurnWatch {
    done: AtomicBool,
    repaint: Mutex<Option<Box<dyn Fn() + Send>>>,
}

struct Finished<'a>(&'a TurnWatch);

impl Drop for Finished<'_> {
    fn drop(&mut self) {
        self.0.done.store(true, Ordering::SeqCst);
        if let Some(repaint) = locked(&self.0.repaint).as_ref() {
            repaint();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Step {
    Exit(EditorExit),
    ClearDraft,
    Continue,
}

pub(super) fn exit_for(signal: Signal, buffer: &str) -> Step {
    match signal {
        Signal::Success(line) => Step::Exit(EditorExit::Submitted(line)),
        Signal::CtrlD => Step::Exit(EditorExit::Closed),
        Signal::HostCommand(command) if command == BACK_OUT => {
            Step::Exit(EditorExit::BackOut(buffer.to_string()))
        }
        Signal::HostCommand(command) if command == QUEUE_CTRL_C => match queue_ctrl_c(buffer) {
            QueueCtrlC::ClearDraft => Step::ClearDraft,
            QueueCtrlC::CancelTurn => Step::Exit(EditorExit::Cancelled),
        },
        _ => Step::Continue,
    }
}

fn read_whole_input() -> Vec<u8> {
    let mut bytes = read_input();
    for _ in 0..JOIN_TRIES {
        if !incomplete(&bytes) || poll_input(JOIN_POLL_MS) != Input::Ready {
            break;
        }
        bytes.extend(read_input());
    }
    bytes
}

fn drain_input() -> String {
    let mut bytes = Vec::new();
    while poll_input(0) == Input::Ready {
        let chunk = read_input();
        if chunk.is_empty() {
            break;
        }
        bytes.extend(chunk);
    }
    match typed(&bytes) {
        Typed::Open(text) => text,
        Typed::Ignore => String::new(),
    }
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
            Ok(signal) => match exit_for(signal, editor.current_buffer_contents()) {
                Step::Exit(exit) => return exit,
                Step::ClearDraft => editor.run_edit_commands(&[EditCommand::Clear]),
                Step::Continue => {}
            },
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
        let watch = TurnWatch::default();
        let (result, mut queue) = thread::scope(|scope| {
            let finished = Finished(&watch);
            let listener = scope.spawn(|| self.listen(&watch));
            let result = run();
            drop(finished);
            (result, listener.join().unwrap_or_default())
        });
        queue.typed.push_str(&drain_input());
        self.cancel.store(false, Ordering::SeqCst);
        self.gate.resume();
        (result, queue)
    }

    fn listen(&self, watch: &TurnWatch) -> Queue {
        let mut queue = Queue::default();
        while !watch.done.load(Ordering::SeqCst) {
            match poll_input(POLL_MS) {
                Input::Idle => continue,
                Input::Closed => break,
                Input::Ready => {}
            }
            let bytes = read_whole_input();
            if bytes.is_empty() {
                thread::sleep(Duration::from_millis(POLL_MS as u64));
            } else if let Typed::Open(seed) = typed(&bytes) {
                if watch.done.load(Ordering::SeqCst) {
                    queue.typed.push_str(&seed);
                } else {
                    queue = self.edit(queue, &seed, watch);
                }
            }
        }
        queue
    }

    fn instant_route(&self, text: &str) -> Option<Route> {
        Some(route_input(
            text,
            self.monitor.status().unread,
            self.empty_submit,
        ))
        .filter(Route::runs_mid_turn)
    }

    fn edit(&self, queue: Queue, seed: &str, watch: &TurnWatch) -> Queue {
        self.gate.pause();
        wait_settled(self.gate);
        let mut editor = build_queue_editor(self.edit_mode);
        let signal = editor.repaint_signal();
        *locked(&watch.repaint) = Some(Box::new(move || signal.request_repaint()));
        if !seed.is_empty() {
            editor.run_edit_commands(&[EditCommand::InsertString(seed.to_string())]);
        }
        let prompt = QueuePrompt {
            monitor: self.monitor,
            bar: self.bar,
            queued: queue.entries.len(),
            held: queue.held,
            done: &watch.done,
        };
        let exit = read_exit(&mut editor, &prompt);
        locked(&watch.repaint).take();
        let exit = match exit {
            EditorExit::Submitted(text) => match self.instant_route(&text) {
                Some(route) => {
                    warn_if_err(
                        run_local(route, self.local, || {
                            self.monitor.refresh(true);
                        }),
                        "Failed to run the command",
                    );
                    EditorExit::Submitted(String::new())
                }
                None => EditorExit::Submitted(text),
            },
            other => other,
        };
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
    fn enter_submits_the_line() {
        assert_eq!(
            exit_for(Signal::Success("hi".into()), ""),
            Step::Exit(EditorExit::Submitted("hi".into()))
        );
    }

    #[test]
    fn queue_ctrl_c_clears_a_draft() {
        assert_eq!(
            exit_for(Signal::HostCommand(QUEUE_CTRL_C.into()), "x"),
            Step::ClearDraft
        );
    }

    #[test]
    fn queue_ctrl_c_on_an_empty_draft_cancels() {
        assert_eq!(
            exit_for(Signal::HostCommand(QUEUE_CTRL_C.into()), ""),
            Step::Exit(EditorExit::Cancelled)
        );
    }

    #[test]
    fn ctrl_d_closes() {
        assert_eq!(exit_for(Signal::CtrlD, "x"), Step::Exit(EditorExit::Closed));
    }

    #[test]
    fn ctrl_q_backs_out_with_the_draft() {
        assert_eq!(
            exit_for(Signal::HostCommand(BACK_OUT.into()), "draft"),
            Step::Exit(EditorExit::BackOut("draft".into()))
        );
    }

    #[test]
    fn other_host_commands_keep_the_editor_open() {
        assert_eq!(
            exit_for(Signal::HostCommand("other".into()), ""),
            Step::Continue
        );
        assert_eq!(
            exit_for(Signal::ExternalBreak(String::new()), ""),
            Step::Continue
        );
    }
}
