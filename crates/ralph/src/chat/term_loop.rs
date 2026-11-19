use crate::chat::cell::{self, ShellState};
use crate::chat::hold::{self, Held};
use crate::chat::inbox::Display;
use crate::chat::keys::Keys;
use crate::chat::prompt::{screen_columns, ChatPrompt, Submitted};
use crate::chat::queue::{
    decide, draft_with_queue, draft_with_typed, next_batch, unsent, unsent_texts, Decision, Queue,
};
use crate::chat::refusal::{refuse_or_exit, restore_prompt};
use crate::chat::schedule::{approve_proposed, Keyboard};
use crate::chat::send_it::{dispatch, held_item, tick, took_over, Active};
use crate::chat::session::{print_exit_notice, route_queued, save_unsent};
use crate::chat::switch::{chat_exit, check as check_switch, ChatExit, Leave};
use crate::chat::{thread, thread_cmd};
use crate::signal;
use ralph_core::chat_cell::CellLimits;
use ralph_core::chat_close::LoopExit;
use ralph_core::chat_hold::HoldFilter;
use ralph_core::chat_local::CommandRecord;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::{Ended, SendItCommand};
use ralph_core::chat_status::Route;
use ralph_core::chat_suggestion::{
    auto_hold_notice, turn_end, Next, Prefill, SuggestionRecord, TurnEnd,
};
use ralph_core::chat_switch::{SwitchArgv, SwitchTarget};
use ralph_core::chat_thread::ThreadName;
use ralph_core::chat_thread_cmd::ThreadOp;
use ralph_core::program_status::Phase;
use ralph_core::worker::Report;
use ralph_server::chat::memory::Store;
use ralph_server::chat::session::{phase_after, trigger_for, At, Session};
use ralph_server::chat::suggestion::SuggestionCell;
use ralph_server::chat::turn::{draft_after, TurnOutcome};
use ralph_server::warn::warn_if_err;
use reedline::{EditCommand, Reedline};
use std::error::Error;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(super) struct TerminalLoop<'a, 's> {
    pub(super) session: &'a Session<'s>,
    pub(super) prompt: &'a ChatPrompt,
    pub(super) store: &'a Store,
    pub(super) run: &'a str,
    pub(super) suggestions: &'a SuggestionCell,
    pub(super) shown: &'a SuggestionCell,
    pub(super) watch_timeout: Duration,
    pub(super) auto_hold: bool,
    pub(super) color: bool,
    pub(super) display: &'a Display<'a>,
    pub(super) idle: &'a AtomicBool,
    pub(super) store_dir: &'a Path,
    pub(super) thread: Option<&'a ThreadName>,
    pub(super) switch_argv: SwitchArgv,
    pub(super) shell: ShellState,
    pub(super) keys: &'a Keys<'a>,
    pub(super) cell_limits: CellLimits,
    pub(super) in_thread: bool,
    pub(super) shown_plan: Option<String>,
    pub(super) editor: Reedline,
    pub(super) pending: Option<Submitted>,
    pub(super) sending: Option<Active>,
    pub(super) offered: Option<(String, u64)>,
    pub(super) carried: Vec<String>,
    pub(super) leaving: Option<ChatExit>,
}

pub(super) struct Ending {
    pub(super) outcome: TurnOutcome,
    pub(super) draft: Option<String>,
    pub(super) queue: Queue,
    pub(super) drained: Vec<Report>,
}

#[derive(PartialEq, Eq)]
pub(super) enum Flow {
    Next,
    Stop,
}

impl TerminalLoop<'_, '_> {
    pub(super) fn queue_next(&mut self) {
        if let Some((text, rest)) = next_batch(std::mem::take(&mut self.carried)) {
            self.carried = rest;
            self.pending = Some(route_queued(text, &mut self.editor, self.prompt));
        }
    }

    pub(super) fn note(&self, record: &CommandRecord, output: &str) -> Option<usize> {
        record
            .row(output)
            .and_then(|row| thread_cmd::append_row(self.store, &row))
    }

    fn run_turn(
        &self,
        trigger: Trigger,
        sent: &str,
    ) -> Result<(TurnOutcome, Queue), Box<dyn Error>> {
        let session = self.session;
        let logged = [sent.to_owned()];
        let (result, queue) = session.snapshots.around(|| {
            self.keys
                .during(|cancel| session.turn.run(trigger, &logged, sent, cancel))
        });
        Ok((result?, queue))
    }

    pub(super) fn send(&mut self, message: String) -> Result<Ending, Box<dyn Error>> {
        crate::render::image::print_attachments(&message);
        self.session.core.set_phase(Phase::Cell);
        let (outcome, queue) =
            match cell::prepare(&message, self.cell_limits, &mut self.shell, self.color) {
                Ok(prepared) => {
                    if let Some(shown) = &prepared.shown {
                        cell::show(&mut std::io::stdout(), self.display, shown);
                    }
                    self.session.core.set_phase(Phase::Turn);
                    self.run_turn(trigger_for(self.sending.is_some()), &prepared.sent)?
                }
                Err(outcome) => (outcome, Queue::default()),
            };
        Ok(Ending {
            outcome,
            draft: draft_after(outcome, message),
            queue,
            drained: Vec::new(),
        })
    }

    pub(super) fn send_it(
        &mut self,
        command: SendItCommand,
        typed_at_prompt: bool,
        record: &CommandRecord,
    ) -> Result<Option<Ending>, Box<dyn Error>> {
        let session = self.session;
        let at = At {
            shown_plan: self.shown_plan.as_deref(),
            typed_at_prompt,
            width: screen_columns(),
        };
        let mut keyboard = Keyboard {
            editor: &self.prompt.editor,
        };
        let reply = session.send_it(command, &at, &mut keyboard);
        let Some(proposal) = reply.proposal else {
            println!("{}", reply.screen.for_terminal(self.color));
            if reply.note {
                self.note(record, &reply.screen.text);
            }
            if let Some(shown) = reply.shown {
                self.shown_plan = shown;
            }
            if let Some(started) = reply.started {
                let mut driven = dispatch(&session.send_it, started, &session.core);
                (self.sending, self.pending) =
                    (driven.active.take(), driven.enter(&mut self.editor));
            }
            return Ok(None);
        };
        session.core.set_phase(Phase::Turn);
        let (outcome, queue) = self.run_turn(Trigger::User, &proposal.sent)?;
        let screen = session.proposed(proposal, screen_columns());
        println!("{}", screen.for_terminal(self.color));
        self.shown_plan = screen.shown;
        Ok(Some(Ending {
            outcome,
            draft: None,
            queue,
            drained: Vec::new(),
        }))
    }

    pub(super) fn thread_op(
        &mut self,
        op: Result<ThreadOp, String>,
        line: &str,
        record: &CommandRecord,
    ) -> Result<Flow, Box<dyn Error>> {
        let merged = thread_cmd::merging(&op);
        let main = thread::config_root(self.store_dir);
        let ran = op.and_then(|op| {
            let env = thread_cmd::ThreadEnv {
                store_dir: self.store_dir,
                in_thread: self.in_thread,
                width: screen_columns(),
                display: self.display,
            };
            thread_cmd::run(&op, &env, &mut thread_cmd::confirm, &|name| {
                thread::running_ids(&main, name)
            })
        });
        match ran {
            Ok(text) => {
                println!("{text}");
                let id = self.note(record, &text);
                thread_cmd::land(&main, merged, id);
                self.session.core.refresh(true);
                self.queue_next();
            }
            Err(refusal) => {
                println!("{refusal}");
                self.note(record, &refusal);
                if refuse_or_exit(&mut self.editor, line, std::mem::take(&mut self.carried))? {
                    self.leaving = Some(chat_exit(LoopExit::ThreadOpRefused));
                    return Ok(Flow::Stop);
                }
            }
        }
        Ok(Flow::Next)
    }

    pub(super) fn shell(&mut self, line: &str) -> Result<Flow, Box<dyn Error>> {
        let monitor = &self.session.core;
        monitor.set_phase(Phase::Cell);
        let ran = cell::run_local(line, &mut self.shell, self.color);
        monitor.set_phase(Phase::Prompt);
        monitor.refresh(true);
        match ran {
            Ok(shown) => {
                cell::show(&mut std::io::stdout(), self.display, &shown);
                self.queue_next();
            }
            Err(TurnOutcome::Exit) => {
                let lost = unsent_texts(&self.carried, self.editor.current_buffer_contents());
                let saved = save_unsent(&mut self.editor, &lost);
                print_exit_notice(lost.len(), saved);
                self.editor.sync_history()?;
                self.leaving = Some(chat_exit(LoopExit::ShellCell));
                return Ok(Flow::Stop);
            }
            Err(_) => restore_prompt(&mut self.editor, line, std::mem::take(&mut self.carried)),
        }
        Ok(Flow::Next)
    }

    pub(super) fn hold(&mut self, filter: &HoldFilter) -> Result<Flow, Box<dyn Error>> {
        let session = self.session;
        self.idle.store(true, Ordering::SeqCst);
        session.core.set_phase(Phase::Turn);
        let held = hold::run(filter, &session.core, &self.prompt.bar);
        self.idle.store(false, Ordering::SeqCst);
        match held {
            Held::Item(position) => {
                let driven =
                    held_item(&session.send_it, &mut self.sending, position, &session.core)?;
                self.pending = driven.enter(&mut self.editor);
            }
            Held::Left => {
                session.core.set_phase(Phase::Prompt);
                if let Some(active) = self.sending.take() {
                    took_over(&session.send_it, &active).enter(&mut self.editor);
                }
            }
            Held::Quit => {
                self.leaving = Some(chat_exit(LoopExit::HeldQuit));
                return Ok(Flow::Stop);
            }
        }
        Ok(Flow::Next)
    }

    pub(super) fn switch(
        &mut self,
        parsed: Result<SwitchTarget, String>,
        line: &str,
    ) -> Result<Flow, Box<dyn Error>> {
        let leave = Leave {
            store_dir: self.store_dir,
            current: self.thread,
            monitor: &self.session.core,
        };
        let target = match parsed.and_then(|target| check_switch(&leave, &target).map(|()| target))
        {
            Ok(target) => target,
            Err(refusal) => {
                println!("{refusal}");
                if refuse_or_exit(&mut self.editor, line, std::mem::take(&mut self.carried))? {
                    self.leaving = Some(chat_exit(LoopExit::SwitchRefused));
                    return Ok(Flow::Stop);
                }
                return Ok(Flow::Next);
            }
        };
        let lost = unsent_texts(&self.carried, self.editor.current_buffer_contents());
        let saved = save_unsent(&mut self.editor, &lost);
        if !saved {
            if signal::is_terminated() {
                print_exit_notice(lost.len(), saved);
                eprintln!("could not save the unsent text; exiting on signal.");
                self.leaving = Some(chat_exit(LoopExit::SwitchSaveFailed));
                return Ok(Flow::Stop);
            }
            eprintln!("could not save the unsent text; the switch is cancelled.");
            restore_prompt(&mut self.editor, line, std::mem::take(&mut self.carried));
            return Ok(Flow::Next);
        }
        print_exit_notice(lost.len(), saved);
        warn_if_err(self.editor.sync_history(), "Failed to save chat history");
        self.idle.store(false, Ordering::SeqCst);
        *self
            .session
            .turn
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.leaving = Some(ChatExit::Switch(target, self.switch_argv.clone()));
        Ok(Flow::Stop)
    }

    pub(super) fn after_turn(&mut self, ending: Ending) -> Result<Flow, Box<dyn Error>> {
        let Ending {
            outcome,
            draft,
            mut queue,
            drained,
        } = ending;
        let session = self.session;
        let monitor = &session.core;
        queue
            .entries
            .splice(0..0, std::mem::take(&mut self.carried));
        monitor.set_phase(phase_after(outcome));
        monitor.refresh(true);
        if outcome == TurnOutcome::Exit {
            let lost = unsent(&queue);
            let saved = save_unsent(&mut self.editor, &lost);
            print_exit_notice(lost.len(), saved);
            self.editor.sync_history()?;
            self.leaving = Some(chat_exit(LoopExit::TurnExit));
            return Ok(Flow::Stop);
        }
        let asked = approve_proposed(&monitor.schedule, self.watch_timeout);
        if let Some(active) = self.sending.take() {
            let unread = monitor.inbox.peek().unwrap_or_default();
            let ended = Ended {
                file: None,
                replied: outcome == TurnOutcome::Replied,
                asked,
                typed: queue.held || !queue.typed.is_empty(),
                queued: &queue.entries,
                cost_usd: session.turn.take_cost().unwrap_or_default(),
                step_result: None,
                drained: &drained,
                unread: &unread,
                answered: &[],
                watched: None,
                refused: None,
            };
            let ticked = tick(&session.send_it, &active, ended);
            let driven = dispatch(&session.send_it, ticked, monitor);
            self.sending = driven.active;
            if driven.route.is_some() {
                self.pending = driven.route.map(Submitted::internal);
                return Ok(Flow::Next);
            }
            self.suggestions.clear();
            if let Some(line) = driven.prefill {
                self.suggestions.set(line);
            }
        }
        let suggestion = self.suggestions.take();
        let typed = std::mem::take(&mut queue.typed);
        let (draft, suggestion) = match decide(outcome, queue) {
            Decision::Send { text, rest } => {
                self.carried = rest;
                self.pending = Some(route_queued(text, &mut self.editor, self.prompt));
                (draft, None)
            }
            Decision::Keep(entries) => (draft_with_queue(draft, &entries), suggestion),
        };
        let draft = draft_with_typed(draft, &typed);
        let replied = outcome == TurnOutcome::Replied && !asked;
        self.shown.clear();
        self.offered = None;
        let turn_number = session.turn.process_turns.load(Ordering::SeqCst);
        let suggested = suggestion.clone();
        let next = turn_end(TurnEnd {
            auto_hold: self.auto_hold,
            replied,
            draft,
            suggestion,
            status: &monitor.status(),
        });
        match next {
            Next::Hold(filter) => {
                print!("{}", self.prompt.dim(&auto_hold_notice(&filter)));
                if let Some(suggested) = suggested {
                    warn_if_err(
                        self.store.append_suggestion(&SuggestionRecord::held(
                            chrono::Utc::now().to_rfc3339(),
                            self.run,
                            turn_number,
                            &suggested,
                        )),
                        "Failed to record suggestion outcome",
                    );
                }
                self.pending = Some(Submitted::internal(Route::Hold(filter)));
            }
            Next::Prefill(fill) => {
                let (Prefill::Draft(text) | Prefill::Suggestion(text)) = &fill;
                self.editor
                    .run_edit_commands(&[EditCommand::InsertString(text.clone())]);
                if let Prefill::Suggestion(text) = fill {
                    self.offered = Some((text.clone(), turn_number));
                    self.shown.set(text);
                }
            }
            Next::Nothing => {}
        }
        Ok(Flow::Next)
    }
}
