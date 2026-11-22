use crate::chat::hold_progress::git_out;
use crate::chat::inbox::{self, Display};
use crate::chat::keys::Keys;
use crate::chat::prompt::{screen_columns, Submitted};
use crate::chat::queue::Queue;
use crate::chat::term_loop::run_turn;
use ralph_core::chat_hold::HoldFilter;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::{
    awaits_check, briefed_turn, done_block, done_notice, judge_echo, judge_prompt, resolve_check,
    run_end_prefill, running_line, step_line, stop_block, verify_worker, Driving, Ended, Next,
    StopReason, Ticked,
};
use ralph_core::chat_status::Route;
use ralph_core::chat_suggestion::driven_fill;
use ralph_core::inbox::{Position, Selection};
use ralph_core::program_status::Phase;
use ralph_core::worker::Report;
use ralph_server::chat::monitor::Monitor;
use ralph_server::chat::port::{SessionPort, TurnCall};
use ralph_server::chat::send_it::SendIt;
use ralph_server::chat::turn::TurnOutcome;
use ralph_server::watches::{locked, OutcomeSlot};
use reedline::{EditCommand, Reedline};
use std::io;

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

pub(super) fn tick(send_it: &SendIt, active: &Active, ended: Ended) -> Ticked {
    send_it.tick(&active.driving, active.watched.as_ref(), ended)
}

pub(super) fn held_item(
    send_it: &SendIt,
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
    let ticked = send_it.settle(&active.driving, &outcome, &reports);
    log_shown(send_it, &ticked);
    let mut driven = route(send_it, ticked, monitor);
    if driven.route.is_none() {
        monitor.set_phase(Phase::Prompt);
    }
    *sending = driven.active.take();
    Ok(driven)
}

pub(super) fn brief_unread(
    send_it: &SendIt,
    sending: &mut Option<Active>,
    message: String,
) -> String {
    let Some(active) = sending.as_mut().filter(|active| !active.driving.briefed) else {
        return message;
    };
    let Some(file) = send_it.load().ok().flatten() else {
        return message;
    };
    let (driving, text) = briefed_turn(&file.plan, active.driving.clone(), &message);
    active.driving = driving;
    text
}

pub(super) fn dispatch(send_it: &SendIt, ticked: Ticked, monitor: &Monitor) -> Driven {
    log_shown(send_it, &ticked);
    route(send_it, ticked, monitor)
}

fn log_shown(send_it: &SendIt, ticked: &Ticked) {
    for line in ticked.shown_lines() {
        send_it.log_notice(line);
    }
}

fn route(send_it: &SendIt, ticked: Ticked, monitor: &Monitor) -> Driven {
    for line in ticked.shown_lines() {
        send_it.show(line);
    }
    let plan = send_it.load().ok().flatten().map(|file| file.plan);
    let driving = ticked.driving;
    match ticked.next {
        Next::Send(line) => {
            if let Some(plan) = &plan {
                send_it.show(running_line(plan, &driving));
            }
            send_it.show(&line);
            Driven::keep(driving, Route::Send(line), None)
        }
        Next::Judge => match plan.and_then(|plan| {
            let prompt = judge_prompt(&plan, driving.progress)?;
            let echo = judge_echo(&plan, driving.progress);
            let (driving, prompt) = briefed_turn(&plan, driving.clone(), &prompt);
            Some((driving, prompt, echo))
        }) {
            Some((driving, prompt, echo)) => {
                send_it.show(echo);
                Driven::keep(driving, Route::Send(prompt), None)
            }
            None => stopped(send_it, StopReason::PlanChanged(None), &driving),
        },
        Next::Hold(worker) => Driven::keep(driving, Route::Hold(HoldFilter::Source(worker)), None),
        Next::Watch(command) | Next::Check(command) => {
            if let Some(plan) = &plan {
                send_it.show(running_line(plan, &driving));
                if let Some(step) = plan.steps.get(driving.progress.step) {
                    send_it.show(step_line(plan, driving.progress.step, step));
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
                Err(reason) => return stopped(send_it, reason, &driving),
            };
            let id = spec.id.clone();
            match monitor.watches.start(spec) {
                Ok(slot) => Driven::keep(driving, Route::Hold(HoldFilter::Source(id)), Some(slot)),
                Err(error) => {
                    send_it.show(&error);
                    let step = driving.progress.step + 1;
                    stopped(
                        send_it,
                        StopReason::CheckNotStarted {
                            step,
                            error: error.to_string(),
                        },
                        &driving,
                    )
                }
            }
        }
        Next::Stop(reason) => stopped(send_it, reason, &driving),
        Next::Done => match &plan {
            Some(plan) => ended(
                send_it,
                &done_notice(plan, &driving.run),
                &done_block(plan, &driving, screen_columns()),
                &driving,
            ),
            None => {
                if let Some(met) = ticked.met.as_deref() {
                    send_it.show(met);
                    send_it.log_notice(met);
                }
                Driven::ended()
            }
        },
        Next::Wait => Driven::ended(),
    }
}

fn ended(send_it: &SendIt, notice: &str, block: &str, driving: &Driving) -> Driven {
    send_it.show(block);
    send_it.log_notice(notice);
    if let Err(error) = send_it.release(&driving.hash, None) {
        send_it.show(error);
    }
    Driven::ended()
}

fn stopped(send_it: &SendIt, reason: StopReason, driving: &Driving) -> Driven {
    let kept = send_it.release(&driving.hash, Some(driving.progress.step + 1));
    let kept_file = kept.as_ref().ok().and_then(Option::as_ref);
    let notice = reason.notice(&driving.run, kept_file);
    send_it.show(stop_block(&reason, kept_file, driving, screen_columns()));
    send_it.log_notice(&notice);
    if let Err(error) = kept {
        send_it.show(error);
    }
    send_it.blocked(&notice);
    Driven {
        prefill: run_end_prefill(&Next::Stop(reason)),
        ..Driven::ended()
    }
}

pub(super) fn took_over(send_it: &SendIt, active: &Active) -> Driven {
    stopped(send_it, StopReason::TookOver, &active.driving)
}

pub(super) struct InboxTurn<'a> {
    pub(super) port: &'a dyn SessionPort,
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
        self.port.set_phase(Phase::Turn);
        let drained = self.monitor.drain(selection)?;
        self.port.refresh();
        let Some(unread) = inbox::read(
            selection,
            &drained,
            display,
            screen_columns(),
            &mut io::stdout(),
        )?
        else {
            self.port.set_phase(Phase::Prompt);
            return Ok(None);
        };
        let message = brief_unread(send_it, sending, unread.message);
        let call = TurnCall {
            trigger: Trigger::Worker,
            logged: &unread.logged,
            sent: &message,
        };
        let (outcome, queue) = run_turn(self.port, &call, |run| self.keys.during(run).1)?;
        Ok(Some(Read {
            outcome,
            queue,
            drained,
        }))
    }
}

#[cfg(test)]
pub(in crate::chat) mod tests;
