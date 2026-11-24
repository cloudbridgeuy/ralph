use crate::chat::cell::{self, ShellState};
use crate::chat::hold::{self, Held};
use crate::chat::inbox::Display;
use crate::chat::keys::Keys;
use crate::chat::prompt::{next_route, screen_columns, ChatPrompt, Submitted};
use crate::chat::queue::{
    decide, draft_with_queue, draft_with_typed, next_batch, unsent, unsent_texts, Decision, Queue,
};
use crate::chat::refusal::{refuse_or_exit, restore_prompt};
use crate::chat::rerender::redraw_text;
use crate::chat::resize::Resize;
use crate::chat::route::{run_local, LocalRoute};
use crate::chat::schedule::{approve_proposed, Keyboard};
use crate::chat::send_it::{dispatch, held_item, tick, took_over, Active, InboxTurn};
use crate::chat::session::{print_exit_notice, route_queued, save_unsent};
use crate::chat::switch::{chat_exit, check as check_switch, ChatExit, Leave};
use crate::chat::{thread, thread_cmd};
use crate::signal;
use crate::warn::print_above_prompt;
use ralph_core::cancel::CancelToken;
use ralph_core::chat_cell::CellLimits;
use ralph_core::chat_close::LoopExit;
use ralph_core::chat_hold::HoldFilter;
use ralph_core::chat_local::CommandRecord;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::{Ended, SendItCommand, Trust};
use ralph_core::chat_status::Route;
use ralph_core::chat_suggestion::{
    auto_hold_notice, turn_end, Next, Prefill, SuggestionRecord, TurnEnd,
};
use ralph_core::chat_switch::{SwitchArgv, SwitchTarget};
use ralph_core::chat_thread::ThreadName;
use ralph_core::chat_thread_auto::{close_footer, close_names, fresh_closes};
use ralph_core::chat_thread_cmd::ThreadOp;
use ralph_core::program_status::Phase;
use ralph_core::worker::Report;
use ralph_server::chat::memory::Store;
use ralph_server::chat::port::{SessionPort, TurnCall};
use ralph_server::chat::session::{phase_after, trigger_for, At, Session};
use ralph_server::chat::suggestion::SuggestionCell;
use ralph_server::chat::turn::{draft_after, TurnOutcome};
use ralph_server::warn::warn_if_err;
use reedline::{EditCommand, Reedline};
use std::error::Error;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::Duration;

pub(super) struct TerminalLoop<'a> {
    pub(super) session: &'a Session,
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
    pub(super) at_prompt: &'a AtomicBool,
    pub(super) prompt_sink: SyncSender<String>,
    pub(super) inbox_turn: &'a InboxTurn<'a>,
    pub(super) inbox_display: &'a dyn Fn(bool) -> Display<'a>,
    pub(super) local: &'a LocalRoute<'a>,
    pub(super) editor: Reedline,
    pub(super) pending: Option<Submitted>,
    pub(super) sending: Option<Active>,
    pub(super) offered: Option<(String, u64)>,
    pub(super) carried: Vec<String>,
    pub(super) leaving: Option<ChatExit>,
    pub(super) announced: Vec<ThreadName>,
    pub(super) resize: &'a Resize,
}

pub(super) fn run_turn(
    port: &dyn SessionPort,
    call: &TurnCall,
    during: impl FnOnce(&mut dyn FnMut(&CancelToken)) -> Queue,
) -> Result<(TurnOutcome, Queue), Box<dyn Error>> {
    let mut during = Some(during);
    let mut ran = None;
    let mut queue = Queue::default();
    port.around(&mut || {
        if let Some(during) = during.take() {
            queue = during(&mut |cancel| ran = Some(port.turn(call, cancel)));
        }
    });
    let outcome = ran.ok_or("keyboard did not run the turn")??;
    Ok((outcome, queue))
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

impl TerminalLoop<'_> {
    pub(super) fn run(mut self) -> Result<ChatExit, Box<dyn Error>> {
        let session = self.session;
        let monitor = &session.core;
        let settles_closes = !self.in_thread;
        if settles_closes && thread::settle::scan(self.store_dir, &monitor.inbox) {
            session.refresh();
        }
        loop {
            warn_if_err(self.editor.sync_history(), "Failed to save chat history");
            self.redraw_after_resize();
            let typed_at_prompt = self.pending.is_none();
            let submitted = match self.pending.take() {
                Some(submitted) => Some(submitted),
                None => {
                    self.at_prompt.store(true, Ordering::SeqCst);
                    self.idle.store(true, Ordering::SeqCst);
                    print_above_prompt(Some(self.prompt_sink.clone()));
                    let submitted = next_route(&mut self.editor, self.prompt);
                    print_above_prompt(None);
                    self.at_prompt.store(false, Ordering::SeqCst);
                    self.idle.store(false, Ordering::SeqCst);
                    session.set_phase(Phase::Prompt);
                    submitted?
                        .map_err(|exit| {
                            if matches!(exit, ChatExit::Closed) {
                                self.leaving = Some(chat_exit(LoopExit::IdleClose));
                            }
                        })
                        .ok()
                        .inspect(|submitted| {
                            if let Some((suggestion, turn_number)) = self.offered.take() {
                                let record = SuggestionRecord::new(
                                    chrono::Utc::now().to_rfc3339(),
                                    self.run,
                                    turn_number,
                                    &suggestion,
                                    &submitted.line,
                                );
                                warn_if_err(
                                    self.store.append_suggestion(&record),
                                    "Failed to record suggestion outcome",
                                );
                            }
                        })
                }
            };
            let Some(Submitted {
                line: typed_line,
                route,
            }) = submitted
            else {
                break;
            };
            let asked = settles_closes
                && typed_at_prompt
                && typed_line.trim().is_empty()
                && thread::settle::scan(self.store_dir, &monitor.inbox);
            if asked {
                session.refresh();
                if !(matches!(route, Route::ReadInbox(_)) && session.status().unread > 0) {
                    continue;
                }
            }
            let record = CommandRecord::for_route(&route, &typed_line);
            let ending = match route {
                Route::Ignore => continue,
                Route::Shell(line) => match self.shell(&line)? {
                    Flow::Next => continue,
                    Flow::Stop => break,
                },
                Route::Send(message) => self.send(message)?,
                Route::ReadInbox(selection) => {
                    let display = (self.inbox_display)(self.color);
                    let Some(read) = self.inbox_turn.read(
                        &session.send_it,
                        selection,
                        &display,
                        &mut self.sending,
                    )?
                    else {
                        continue;
                    };
                    Ending {
                        outcome: read.outcome,
                        draft: None,
                        queue: read.queue,
                        drained: read.drained,
                    }
                }
                Route::Hold(filter) => match self.hold(&filter)? {
                    Flow::Next => continue,
                    Flow::Stop => break,
                },
                Route::SendIt(command) => match self.send_it(command, typed_at_prompt, &record)? {
                    Some(ending) => ending,
                    None => continue,
                },
                Route::Switch { parsed, line } => match self.switch(parsed, &line)? {
                    Flow::Next => continue,
                    Flow::Stop => break,
                },
                Route::ThreadOp { op, line } => match self.thread_op(op, &line, &record)? {
                    Flow::Next => continue,
                    Flow::Stop => break,
                },
                local => {
                    if let Some(output) = run_local(local, self.local, &mut io::stdout(), || {
                        session.refresh();
                    })? {
                        self.note(&record, &output);
                    }
                    continue;
                }
            };
            if self.after_turn(ending)? == Flow::Stop {
                break;
            }
        }
        Ok(self.leaving.unwrap_or(ChatExit::Quit))
    }

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

    fn redraw_after_resize(&self) {
        let cols = screen_columns();
        if self.resize.take_pending() && self.resize.take_redraw(true, cols) {
            if let Some(text) = redraw_text(self.store_dir, self.local.reply_display, cols) {
                print!("{text}");
            }
        }
    }

    fn run_turn(
        &self,
        trigger: Trigger,
        sent: &str,
    ) -> Result<(TurnOutcome, Queue), Box<dyn Error>> {
        let logged = [sent.to_owned()];
        let call = TurnCall {
            trigger,
            logged: &logged,
            sent,
        };
        run_turn(self.session, &call, |run| self.keys.during(run).1)
    }

    pub(super) fn send(&mut self, message: String) -> Result<Ending, Box<dyn Error>> {
        crate::render::image::print_attachments(&message);
        self.session.set_phase(Phase::Cell);
        let (outcome, queue) =
            match cell::prepare(&message, self.cell_limits, &mut self.shell, self.color) {
                Ok(prepared) => {
                    if let Some(shown) = &prepared.shown {
                        cell::show(&mut std::io::stdout(), self.display, shown);
                    }
                    self.session.set_phase(Phase::Turn);
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
        session.set_phase(Phase::Turn);
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
                mid_turn: false,
                width: screen_columns(),
                display: Some(self.display),
                notice: &|text| println!("{text}"),
                curate: &|prompt, curator| crate::run_curator(prompt, curator),
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
                self.session.refresh();
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
        let session = self.session;
        session.set_phase(Phase::Cell);
        let ran = cell::run_local(line, &mut self.shell, self.color);
        session.set_phase(Phase::Prompt);
        session.refresh();
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
        session.set_phase(Phase::Turn);
        let held = hold::run(filter, &session.core, &self.prompt.bar);
        self.idle.store(false, Ordering::SeqCst);
        match held {
            Held::Item(position) => {
                let driven =
                    held_item(&session.send_it, &mut self.sending, position, &session.core)?;
                self.pending = driven.enter(&mut self.editor);
            }
            Held::Left => {
                session.set_phase(Phase::Prompt);
                if let Some(active) = self.sending.take() {
                    took_over(&session.send_it, &active).enter(&mut self.editor);
                }
            }
            Held::Quit => {
                self.leaving = Some(chat_exit(LoopExit::HeldQuit));
                return Ok(Flow::Stop);
            }
        }
        self.announce_closes();
        Ok(Flow::Next)
    }

    fn announce_closes(&mut self) {
        self.session.refresh();
        let pending = self.session.status().closes;
        let fresh = fresh_closes(&pending, &self.announced);
        if let Some(line) = close_footer(&fresh, screen_columns()) {
            print!("{}", self.prompt.dim(&line));
        }
        self.announced = close_names(&pending);
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
        self.session.end_session();
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
        session.set_phase(phase_after(outcome));
        session.refresh();
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
            let unread = session.unread();
            let ended = Ended {
                file: None,
                trust: Trust::NONE,
                replied: outcome == TurnOutcome::Replied,
                asked,
                typed: queue.held || !queue.typed.is_empty(),
                queued: &queue.entries,
                cost_usd: session.take_cost().unwrap_or_default(),
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
        let turn_number = session.turns();
        let suggested = suggestion.clone();
        let next = turn_end(TurnEnd {
            auto_hold: self.auto_hold,
            replied,
            draft,
            suggestion,
            status: &session.status(),
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

#[cfg(test)]
mod tests;
