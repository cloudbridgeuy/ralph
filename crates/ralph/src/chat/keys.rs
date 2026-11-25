use crate::chat::editor::{
    build_queue_editor, edit_queue_draft, BACK_OUT, OPEN_EDITOR, QUEUE_CTRL_C,
};
use crate::chat::prompt::{retry_interrupted, QueuePrompt};
use crate::chat::queue::{
    at_once, on_exit, switch_verdict, take_notes, Action, EditorExit, MidTurn, Queue,
};
use crate::chat::route::{run_local, LocalRoute};
use crate::chat::sent::SentStyle;
use crate::chat::statusbar::StatusBar;
use crate::chat::switch::{self, Leave};
use crate::chat::terminal::{poll_input, read_input, Input, PartialMode, POLL_MS};
use crate::chat::thread_cmd::append_row;
use crate::signal;
use ralph_core::cancel::CancelToken;
use ralph_core::chat_edit_mode::EditMode as EditModeConfig;
use ralph_core::chat_hold::hold_targets;
use ralph_core::chat_local::CommandRecord;
use ralph_core::chat_status::{route_input, EmptySubmit, Route};
use ralph_core::chat_thread::config_root;
use ralph_core::chat_typed::{incomplete, queue_ctrl_c, typed, QueueCtrlC, Typed};
use ralph_core::worker::{
    blocked_path, key_answer_report, oldest_worker_answer, Choice, GrantsChange, HANDOFF_TIMEOUT,
};
use ralph_server::chat::memory::Store;
use ralph_server::chat::monitor::Monitor;
use ralph_server::config::{saved_grants, GRANTS_FILE};
use ralph_server::subprocess::OutputGate;
use ralph_server::warn::warn_if_err;
use ralph_server::watches::locked;
use reedline::{EditCommand, Reedline, Signal};
use std::io::IsTerminal;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

const SETTLE_LIMIT: Duration = Duration::from_millis(300);
const SETTLE_STEP: Duration = Duration::from_millis(10);
const JOIN_POLL_MS: libc::c_int = 10;
const JOIN_TRIES: usize = 2;

pub(super) struct Keys<'a> {
    pub(super) gate: &'a OutputGate,
    pub(super) monitor: &'a Monitor,
    pub(super) bar: &'a StatusBar,
    pub(super) edit_mode: EditModeConfig,
    pub(super) sent_style: SentStyle,
    pub(super) local: &'a LocalRoute<'a>,
    pub(super) empty_submit: &'a EmptySubmit,
    pub(super) store: &'a Path,
    pub(super) editor: &'a [String],
}

#[derive(Default)]
struct TurnWatch {
    cancel: CancelToken,
    done: AtomicBool,
    repaint: Mutex<Option<Box<dyn Fn() + Send>>>,
}

struct Finished<'a>(&'a TurnWatch);

struct Listening<'a>(&'a OutputGate);

impl Drop for Listening<'_> {
    fn drop(&mut self) {
        self.0.listen(false);
    }
}

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
    OpenEditor,
    Continue,
}

pub(super) fn answer_oldest_worker(monitor: &Monitor, choice: Choice) -> Option<String> {
    let store = &monitor.store_dir;
    let markers: Vec<(String, u128, String)> = monitor
        .status()
        .workers
        .into_iter()
        .filter_map(|worker| {
            let path = blocked_path(store, &worker.id);
            let since = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()?;
            let since = since.duration_since(UNIX_EPOCH).ok()?.as_millis();
            Some((worker.id, since, std::fs::read_to_string(path).ok()?))
        })
        .collect();
    let answer = oldest_worker_answer(
        markers
            .iter()
            .map(|(id, since, marker)| (id.as_str(), *since, marker.as_str())),
        choice,
    )?;
    let before = saved_grants(store);
    let sent = ralph_server::workers::answer_permission(
        store,
        HANDOFF_TIMEOUT,
        &answer.worker,
        &answer.request,
        &answer.option,
    );
    let path = config_root(store).join(GRANTS_FILE).display().to_string();
    Some(key_answer_report(
        &answer.worker,
        choice,
        sent.map_err(|error| error.to_string()),
        GrantsChange {
            before,
            after: saved_grants(store),
            path: &path,
        },
    ))
}

pub(super) fn exit_for(signal: Signal, buffer: &str) -> Step {
    match signal {
        Signal::Success(line) => Step::Exit(EditorExit::Submitted(line)),
        Signal::CtrlD => Step::Exit(EditorExit::Closed),
        Signal::HostCommand(command) if command == OPEN_EDITOR => Step::OpenEditor,
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

impl Keys<'_> {
    fn read_exit(&self, editor: &mut Reedline, prompt: &QueuePrompt) -> EditorExit {
        loop {
            match retry_interrupted(|| editor.read_line(prompt)) {
                Ok(signal) => match exit_for(signal, editor.current_buffer_contents()) {
                    Step::Exit(exit) => return exit,
                    Step::ClearDraft => editor.run_edit_commands(&[EditCommand::Clear]),
                    Step::OpenEditor => warn_if_err(
                        edit_queue_draft(editor, self.store, self.editor),
                        "Editor failed",
                    ),
                    Step::Continue => {}
                },
                Err(_) => return EditorExit::Closed,
            }
        }
    }

    pub(super) fn during<R>(&self, run: impl FnOnce(&CancelToken) -> R) -> (R, Queue) {
        let watch = TurnWatch::default();
        let _armed = signal::arm(&watch.cancel);
        let Some(_mode) = std::io::stdin()
            .is_terminal()
            .then(PartialMode::enter)
            .flatten()
        else {
            return (run(&watch.cancel), Queue::default());
        };
        let (result, mut queue) = thread::scope(|scope| {
            let finished = Finished(&watch);
            let (ready, started) = std::sync::mpsc::sync_channel(0);
            let turn_watch = &watch;
            let listener = scope.spawn(move || {
                self.gate.listen(std::io::stdout().is_terminal());
                let _listening = Listening(self.gate);
                let _ = ready.send(());
                self.listen(turn_watch)
            });
            let _ = started.recv();
            let result = run(&watch.cancel);
            drop(finished);
            (result, listener.join().unwrap_or_default())
        });
        queue.typed.push_str(&drain_input());
        self.gate.resume();
        let (notes, queue) = take_notes(queue);
        let store = Store::new(self.store.to_path_buf());
        for row in &notes {
            append_row(&store, row);
        }
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
                break;
            } else if let Typed::Open(seed) = typed(&bytes) {
                if watch.done.load(Ordering::SeqCst) {
                    queue.typed.push_str(&seed);
                } else {
                    let (next, closed) = self.edit(queue, &seed, watch);
                    queue = next;
                    if closed {
                        break;
                    }
                }
            }
        }
        queue
    }

    fn routed(&self, text: &str) -> Route {
        let status = self.monitor.status();
        route_input(
            text,
            status.unread,
            self.empty_submit,
            &hold_targets(&status),
        )
    }

    fn edit(&self, mut queue: Queue, seed: &str, watch: &TurnWatch) -> (Queue, bool) {
        self.gate.pause();
        wait_settled(self.gate);
        let mut editor = build_queue_editor(self.edit_mode, self.sent_style);
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
        let exit = self.read_exit(&mut editor, &prompt);
        locked(&watch.repaint).take();
        let closed = matches!(exit, EditorExit::Closed);
        if closed {
            self.gate.listen(false);
        }
        let answered = match &exit {
            EditorExit::Submitted(text) => Choice::from_key(text).and_then(|choice| {
                self.gate
                    .answer_oldest(choice)
                    .map(|outcome| outcome.report(self.store))
                    .or_else(|| answer_oldest_worker(self.monitor, choice))
            }),
            _ => None,
        };
        let exit = match (answered, exit) {
            (Some(message), _) => {
                println!("{message}");
                EditorExit::Submitted(String::new())
            }
            (None, EditorExit::Submitted(text)) => match at_once(self.routed(&text)) {
                MidTurn::Run(route) => {
                    let record = mid_turn_record(&route, &text);
                    let ran = run_local(*route, self.local, &mut std::io::stdout(), || {
                        self.monitor.refresh(true);
                    });
                    let output = ran.as_ref().ok().cloned().flatten();
                    queue.notes.extend(
                        record
                            .zip(output)
                            .and_then(|(record, output)| record.row(&output)),
                    );
                    warn_if_err(ran, "Failed to run the command");
                    EditorExit::Submitted(String::new())
                }
                MidTurn::Check(target) => {
                    let leave = Leave {
                        store_dir: self.store,
                        current: self.local.thread,
                        monitor: self.monitor,
                    };
                    let verdict = switch_verdict(switch::check(&leave, &target));
                    println!("{}", verdict.notice());
                    match verdict.keeps_line() {
                        true => EditorExit::Submitted(text),
                        false => EditorExit::Submitted(String::new()),
                    }
                }
                MidTurn::Queue => EditorExit::Submitted(text),
            },
            (None, other) => other,
        };
        let (queue, action) = on_exit(queue, exit);
        self.gate.resume();
        if action == Action::Cancel {
            watch.cancel.cancel();
        }
        (queue, closed)
    }
}

fn mid_turn_record(route: &Route, line: &str) -> Option<CommandRecord> {
    matches!(route, Route::ThreadOp { .. }).then(|| CommandRecord::for_route(route, line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_turn_gets_its_own_cancel_token() {
        let cancelled = TurnWatch::default();
        let next = TurnWatch::default();
        cancelled.cancel.cancel();
        assert!(!next.cancel.is_cancelled());
    }

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
    fn queue_ctrl_g_opens_the_editor() {
        assert_eq!(
            exit_for(Signal::HostCommand(OPEN_EDITOR.into()), "draft"),
            Step::OpenEditor
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
