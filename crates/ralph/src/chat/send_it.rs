use crate::chat::hold_progress::git_out;
use crate::chat::inbox::{self, Display};
use crate::chat::keys::Keys;
use crate::chat::prompt::{screen_columns, Submitted};
use crate::chat::queue::Queue;
use ralph_core::chat_hold::HoldFilter;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::{
    awaits_check, briefed_turn, done_block, done_notice, judge_echo, judge_prompt, resolve_check,
    run_end_prefill, running_line, settle_check, step_line, stop_block, verify_worker, Driving,
    Ended, Limits, Next, StopReason, Ticked,
};
use ralph_core::chat_status::Route;
use ralph_core::chat_suggestion::driven_fill;
use ralph_core::inbox::{Position, Selection};
use ralph_core::program_status::Phase;
use ralph_core::worker::Report;
use ralph_server::chat::monitor::Monitor;
use ralph_server::chat::send_it::SendIt as ServerSendIt;
use ralph_server::chat::session::Emit;
use ralph_server::chat::snapshots::TurnSnapshots;
use ralph_server::chat::turn::{Turn, TurnOutcome};
use ralph_server::watches::{locked, OutcomeSlot};
use reedline::{EditCommand, Reedline};
use std::io;
use std::ops::Deref;
use std::path::Path;

#[derive(Debug, Clone)]
pub(crate) struct SendIt(ServerSendIt);

impl Deref for SendIt {
    type Target = ServerSendIt;

    fn deref(&self) -> &ServerSendIt {
        &self.0
    }
}

pub(super) struct Active {
    driving: Driving,
    watched: Option<OutcomeSlot>,
}

pub(super) struct Driven {
    pub(super) active: Option<Active>,
    pub(super) route: Option<Route>,
    pub(super) prefill: Option<String>,
}

pub(super) fn prefill_commands(prefill: Option<&str>) -> Vec<EditCommand> {
    prefill
        .map(|line| EditCommand::InsertString(line.to_string()))
        .into_iter()
        .collect()
}

impl Driven {
    pub(super) fn enter(self, editor: &mut Reedline) -> Option<Submitted> {
        // Plain buffer insert on purpose: no dim suggestion styling and no suggestion-ledger row, since this is for check-only stops.
        let fill = driven_fill(editor.current_buffer_contents(), self.prefill.as_deref());
        editor.run_edit_commands(&prefill_commands(fill.as_deref()));
        self.route.map(Submitted::internal)
    }

    fn keep(driving: Driving, route: Route, watched: Option<OutcomeSlot>) -> Self {
        Self {
            active: Some(Active { driving, watched }),
            route: Some(route),
            prefill: None,
        }
    }

    fn reading(route: Route) -> Self {
        Self {
            route: Some(route),
            ..Self::ended()
        }
    }

    fn ended() -> Self {
        Self {
            active: None,
            route: None,
            prefill: None,
        }
    }
}

impl SendIt {
    pub(crate) fn new(store_dir: &Path, limits: Limits, emit: Emit) -> Self {
        Self(ServerSendIt::new(store_dir, limits, emit))
    }

    pub(super) fn tick(&self, active: &Active, ended: Ended) -> Ticked {
        self.0.tick(&active.driving, active.watched.as_ref(), ended)
    }

    pub(super) fn held_item(
        &self,
        sending: &mut Option<Active>,
        position: Position,
        monitor: &Monitor,
    ) -> io::Result<Driven> {
        let read = Driven::reading(Route::ReadInbox(Selection::At(position)));
        let outcome = sending
            .as_ref()
            .filter(|active| awaits_check(&active.driving))
            .and_then(|active| active.watched.as_ref())
            .and_then(|slot| locked(slot).clone());
        let Some(outcome) = outcome else {
            return Ok(read);
        };
        let reports = monitor.drain(Selection::At(position))?;
        monitor.refresh(true);
        let Some(active) = sending.take() else {
            return Ok(read);
        };
        let file = self.load().ok().flatten();
        let ticked = settle_check(&active.driving, file.as_ref(), &outcome, &reports);
        self.log_shown(&ticked);
        let mut driven = self.route(ticked, monitor);
        if driven.route.is_none() {
            monitor.set_phase(Phase::Prompt);
        }
        *sending = driven.active.take();
        Ok(driven)
    }

    pub(super) fn brief_unread(&self, sending: &mut Option<Active>, message: String) -> String {
        let Some(active) = sending.as_mut().filter(|active| !active.driving.briefed) else {
            return message;
        };
        let Some(file) = self.load().ok().flatten() else {
            return message;
        };
        let (driving, text) = briefed_turn(&file.plan, active.driving.clone(), &message);
        active.driving = driving;
        text
    }

    pub(super) fn dispatch(&self, ticked: Ticked, monitor: &Monitor) -> Driven {
        self.log_shown(&ticked);
        self.route(ticked, monitor)
    }

    fn log_shown(&self, ticked: &Ticked) {
        for line in ticked.shown_lines() {
            self.log_notice(line);
        }
    }

    fn route(&self, ticked: Ticked, monitor: &Monitor) -> Driven {
        for line in ticked.shown_lines() {
            self.show(line);
        }
        let plan = self.load().ok().flatten().map(|file| file.plan);
        let driving = ticked.driving;
        match ticked.next {
            Next::Send(line) => {
                if let Some(plan) = &plan {
                    self.show(running_line(plan, &driving));
                }
                self.show(&line);
                Driven::keep(driving, Route::Send(line), None)
            }
            Next::Judge => match plan.and_then(|plan| {
                let prompt = judge_prompt(&plan, driving.progress)?;
                let echo = judge_echo(&plan, driving.progress);
                let (driving, prompt) = briefed_turn(&plan, driving.clone(), &prompt);
                Some((driving, prompt, echo))
            }) {
                Some((driving, prompt, echo)) => {
                    self.show(echo);
                    Driven::keep(driving, Route::Send(prompt), None)
                }
                None => self.stopped(StopReason::PlanChanged, &driving),
            },
            Next::Hold(worker) => {
                Driven::keep(driving, Route::Hold(HoldFilter::Source(worker)), None)
            }
            Next::Watch(command) | Next::Check(command) => {
                if let Some(plan) = &plan {
                    self.show(running_line(plan, &driving));
                    if let Some(step) = plan.steps.get(driving.progress.step) {
                        self.show(step_line(plan, driving.progress.step, step));
                    }
                }
                let worker = plan
                    .as_ref()
                    .and_then(|plan| verify_worker(plan, driving.progress));
                let porcelain =
                    worker.and_then(|_| git_out(None, &["worktree", "list", "--porcelain"]));
                let spec = match resolve_check(
                    driving.progress.step,
                    &command,
                    worker,
                    porcelain.as_deref(),
                ) {
                    Ok(spec) => spec,
                    Err(reason) => return self.stopped(reason, &driving),
                };
                let id = spec.id.clone();
                match monitor.watches.start(spec) {
                    Ok(slot) => {
                        Driven::keep(driving, Route::Hold(HoldFilter::Source(id)), Some(slot))
                    }
                    Err(error) => {
                        self.show(&error);
                        let step = driving.progress.step + 1;
                        self.stopped(
                            StopReason::CheckNotStarted {
                                step,
                                error: error.to_string(),
                            },
                            &driving,
                        )
                    }
                }
            }
            Next::Stop(reason) => self.stopped(reason, &driving),
            Next::Done => match &plan {
                Some(plan) => self.ended(
                    &done_notice(plan, &driving.run),
                    &done_block(plan, &driving, screen_columns()),
                    &driving,
                ),
                None => {
                    if let Some(met) = ticked.met.as_deref() {
                        self.show(met);
                        self.log_notice(met);
                    }
                    Driven::ended()
                }
            },
            Next::Wait => Driven::ended(),
        }
    }

    fn ended(&self, notice: &str, block: &str, driving: &Driving) -> Driven {
        self.show(block);
        self.log_notice(notice);
        if let Err(error) = self.release(&driving.hash, None) {
            self.show(error);
        }
        Driven::ended()
    }

    fn stopped(&self, reason: StopReason, driving: &Driving) -> Driven {
        let kept = self.release(&driving.hash, Some(driving.progress.step + 1));
        let kept_file = kept.as_ref().ok().and_then(Option::as_ref);
        let notice = reason.notice(&driving.run, kept_file);
        self.show(stop_block(&reason, kept_file, driving, screen_columns()));
        self.log_notice(&notice);
        if let Err(error) = kept {
            self.show(error);
        }
        self.blocked(&notice);
        Driven {
            prefill: run_end_prefill(&Next::Stop(reason)),
            ..Driven::ended()
        }
    }

    pub(super) fn took_over(&self, active: &Active) -> Driven {
        self.stopped(StopReason::TookOver, &active.driving)
    }
}

pub(super) struct InboxTurn<'a> {
    pub(super) turn: &'a Turn<'a>,
    pub(super) snapshots: &'a TurnSnapshots,
    pub(super) keys: &'a Keys<'a>,
    pub(super) monitor: &'a Monitor,
}

pub(super) struct Read {
    pub(super) outcome: TurnOutcome,
    pub(super) queue: Queue,
    pub(super) drained: Vec<Report>,
}

impl InboxTurn<'_> {
    pub(super) fn read(
        &self,
        send_it: &SendIt,
        selection: Selection,
        display: &Display,
        sending: &mut Option<Active>,
    ) -> Result<Option<Read>, Box<dyn std::error::Error>> {
        self.monitor.set_phase(Phase::Turn);
        let drained = self.monitor.drain(selection)?;
        self.monitor.refresh(true);
        let Some(unread) = inbox::read(
            selection,
            &drained,
            display,
            screen_columns(),
            &mut io::stdout(),
        )?
        else {
            self.monitor.set_phase(Phase::Prompt);
            return Ok(None);
        };
        let message = send_it.brief_unread(sending, unread.message);
        let (result, queue) = self.snapshots.around(|| {
            self.keys.during(|cancel| {
                self.turn
                    .run(Trigger::Worker, &unread.logged, &message, cancel)
            })
        });
        Ok(Some(Read {
            outcome: result?,
            queue,
            drained,
        }))
    }
}

#[cfg(test)]
pub(in crate::chat) mod tests;
