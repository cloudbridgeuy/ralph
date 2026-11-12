use crate::chat::editor::open_in_editor;
use crate::chat::monitor::Monitor;
use crate::inbox_alert;
use ralph_core::chat_hold::HoldFilter;
use ralph_core::chat_local::notice_record;
use ralph_core::chat_memory::Role;
use ralph_core::chat_send_it::template::{
    fill_template, match_template, render_templates, save_notice, template_name, template_text,
};
use ralph_core::chat_send_it::{
    approve, awaits_check, blocked_alert, blocked_report, check_spec, done_notice, judge_prompt,
    parse_run, plan_hash, propose_refusal, read_run, render_status, running_line, settle_check,
    start_run, step_line, stop_file, tick, write_run, Action, Driving, Ended, Limits, Next,
    RunFile, Seen, StepResult, StopReason, Ticked,
};
use ralph_core::chat_status::{latest_states, Route};
use ralph_core::inbox::{Position, Selection};
use ralph_core::program_status::Phase;
use ralph_core::worker_check::logged_states;
use ralph_server::chat::memory::Store;
use ralph_server::warn::warn_if_err;
use ralph_server::watches::{locked, OutcomeSlot};
use ralph_server::workers::running_workers;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const RUN_DIR: &str = "send-it";
const RUN_FILE: &str = "run.toml";
const TEMPLATE_DIR: &str = "templates";
const NO_RUN: &str = "No send-it run. Start one with /send-it <instructions>.";
const NOT_PROPOSED: &str =
    "No new run plan was stored. Try /send-it again with clearer instructions.";

fn unreadable(path: &Path, error: io::Error) -> String {
    format!("Could not read {}: {error}", path.display())
}

#[derive(Debug, Clone)]
pub(crate) struct SendIt {
    log: PathBuf,
    path: PathBuf,
    templates: PathBuf,
    limits: Limits,
    refused: Arc<Mutex<Option<String>>>,
    judged: Arc<Mutex<Option<StepResult>>>,
    answered: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Screen {
    pub(crate) text: String,
    pub(crate) shown: Option<String>,
}

impl Screen {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            shown: None,
        }
    }
}

pub(super) struct Active {
    driving: Driving,
    watched: Option<OutcomeSlot>,
}

pub(super) struct Driven {
    pub(super) active: Option<Active>,
    pub(super) route: Option<Route>,
}

impl Driven {
    fn keep(driving: Driving, route: Route, watched: Option<OutcomeSlot>) -> Self {
        Self {
            active: Some(Active { driving, watched }),
            route: Some(route),
        }
    }

    fn ended() -> Self {
        Self {
            active: None,
            route: None,
        }
    }
}

pub(super) fn worker_states(monitor: &Monitor) -> (HashMap<String, String>, Vec<String>) {
    let mut last = Store::new(monitor.store_dir.clone())
        .load()
        .map(|(entries, _)| logged_states(&entries))
        .unwrap_or_default();
    last.extend(locked(&monitor.seen).clone());
    last.extend(latest_states(&monitor.inbox.peek().unwrap_or_default()));
    let running = running_workers(&monitor.store_dir)
        .into_iter()
        .map(|worker| worker.id)
        .collect();
    (last, running)
}

impl SendIt {
    pub(crate) fn new(store_dir: &Path, limits: Limits) -> Self {
        Self {
            log: store_dir.to_path_buf(),
            path: store_dir.join(RUN_DIR).join(RUN_FILE),
            templates: store_dir.join(RUN_DIR).join(TEMPLATE_DIR),
            limits,
            refused: Arc::default(),
            judged: Arc::default(),
            answered: Arc::default(),
        }
    }

    fn load(&self) -> Result<Option<RunFile>, String> {
        match fs::read_to_string(&self.path) {
            Ok(text) => read_run(&text, self.limits).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(unreadable(&self.path, error)),
        }
    }

    fn save(&self, file: &RunFile) -> Result<(), String> {
        let text = write_run(file)?;
        self.path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&self.path, text))
            .map_err(|error| format!("Could not write {}: {error}", self.path.display()))
    }

    pub(crate) fn allowed(&self) -> Option<Vec<Action>> {
        self.load()
            .ok()
            .flatten()
            .filter(RunFile::is_approved)
            .map(|file| file.plan.allowed)
    }

    pub(crate) fn refuse(&self, tool: String) {
        *locked(&self.refused) = Some(tool);
    }

    pub(crate) fn answer(&self, worker: String) {
        locked(&self.answered).push(worker);
    }

    pub(crate) fn record(&self, result: StepResult) {
        *locked(&self.judged) = Some(result);
    }

    pub(crate) fn refusal(&self) -> Option<String> {
        propose_refusal(self.load().ok().flatten().as_ref())
    }

    pub(crate) fn current_hash(&self) -> Option<String> {
        self.load().ok().flatten().map(|file| plan_hash(&file.plan))
    }

    pub(crate) fn propose(&self, args: &Value) -> Result<String, String> {
        if let Some(refusal) = self.refusal() {
            return Err(refusal);
        }
        let plan = parse_run(args, self.limits)?;
        let hash = plan_hash(&plan);
        self.save(&RunFile {
            plan,
            approved: None,
            stopped: None,
        })?;
        Ok(format!(
            "Stored run plan {hash}. Stop here: ralph shows the plan and the person approves it with /send-it go."
        ))
    }

    pub(crate) fn status(&self) -> Screen {
        match self.load() {
            Ok(Some(file)) => Screen {
                text: render_status(&file, super::prompt::screen_columns()),
                shown: Some(plan_hash(&file.plan)),
            },
            Ok(None) => Screen::text(NO_RUN),
            Err(text) => Screen::text(text),
        }
    }

    pub(crate) fn proposed(&self, before: Option<String>) -> Screen {
        let screen = self.status();
        let stored_new = match &screen.shown {
            Some(hash) => before.as_ref() != Some(hash),
            None => screen.text != NO_RUN,
        };
        if stored_new {
            screen
        } else {
            Screen::text(NOT_PROPOSED)
        }
    }

    pub(crate) fn go(
        &self,
        shown: Option<&str>,
        typed_at_prompt: bool,
        seen: &Seen,
    ) -> (String, Option<Ticked>) {
        let file = match self.load() {
            Ok(Some(file)) => file,
            Ok(None) => return (NO_RUN.to_string(), None),
            Err(text) => return (text, None),
        };
        locked(&self.answered).clear();
        let approved = approve(&file, shown, typed_at_prompt)
            .and_then(|approved| self.save(&approved).map(|()| approved));
        match approved {
            Ok(approved) => (
                format!("Plan {} approved. Running it.", plan_hash(&approved.plan)),
                start_run(&approved, seen),
            ),
            Err(text) => (text, None),
        }
    }

    pub(super) fn tick(&self, active: &Active, ended: Ended) -> Ticked {
        let file = self.load().ok().flatten();
        let watched = active
            .watched
            .as_ref()
            .and_then(|slot| locked(slot).clone());
        let refused = locked(&self.refused).take();
        let judged = locked(&self.judged).take();
        let answered = std::mem::take(&mut *locked(&self.answered));
        tick(
            &active.driving,
            &Ended {
                file: file.as_ref(),
                watched: watched.as_ref(),
                refused: refused.as_deref(),
                answered: &answered,
                step_result: judged.as_ref(),
                ..ended
            },
        )
    }

    pub(super) fn held_item(
        &self,
        sending: &mut Option<Active>,
        position: Position,
        monitor: &Monitor,
    ) -> io::Result<Option<Route>> {
        let read = Route::ReadInbox(Selection::At(position));
        let Some(active) = sending.take() else {
            return Ok(Some(read));
        };
        let outcome = active
            .watched
            .as_ref()
            .and_then(|slot| locked(slot).clone())
            .filter(|_| awaits_check(&active.driving));
        let Some(outcome) = outcome else {
            *sending = Some(active);
            return Ok(Some(read));
        };
        let reports = monitor.drain(Selection::At(position))?;
        monitor.refresh(true);
        let file = self.load().ok().flatten();
        let ticked = settle_check(&active.driving, file.as_ref(), &outcome, &reports);
        if let Some(met) = &ticked.met {
            self.log_notice(met);
        }
        let driven = self.dispatch(ticked, monitor);
        if driven.route.is_none() {
            monitor.set_phase(Phase::Prompt);
        }
        *sending = driven.active;
        Ok(driven.route)
    }

    pub(super) fn dispatch(&self, ticked: Ticked, monitor: &Monitor) -> Driven {
        if let Some(met) = &ticked.met {
            println!("{met}");
        }
        let plan = self.load().ok().flatten().map(|file| file.plan);
        let driving = ticked.driving;
        match ticked.next {
            Next::Send(line) => {
                if let Some(plan) = &plan {
                    println!("{}", running_line(plan, &driving));
                }
                println!("{line}");
                Driven::keep(driving, Route::Send(line), None)
            }
            Next::Judge => match plan.and_then(|plan| judge_prompt(&plan, driving.progress)) {
                Some(prompt) => {
                    println!("{prompt}");
                    Driven::keep(driving, Route::Send(prompt), None)
                }
                None => self.stopped(StopReason::PlanChanged, &driving, monitor),
            },
            Next::Hold(worker) => {
                Driven::keep(driving, Route::Hold(HoldFilter::Source(worker)), None)
            }
            Next::Watch(command) | Next::Check(command) => {
                if let Some(plan) = &plan {
                    println!("{}", running_line(plan, &driving));
                    if let Some(step) = plan.steps.get(driving.progress.step) {
                        println!("{}", step_line(plan, driving.progress.step, step));
                    }
                }
                let spec = check_spec(driving.progress.step, &command);
                let id = spec.id.clone();
                match monitor.watches.start(spec) {
                    Ok(slot) => {
                        Driven::keep(driving, Route::Hold(HoldFilter::Source(id)), Some(slot))
                    }
                    Err(error) => {
                        println!("{error}");
                        let step = driving.progress.step + 1;
                        self.stopped(StopReason::check_failed(step), &driving, monitor)
                    }
                }
            }
            Next::Stop(reason) => self.stopped(reason, &driving, monitor),
            Next::Done => match &plan {
                Some(plan) => self.ended(&done_notice(plan, &driving.run), &driving),
                None => Driven::ended(),
            },
            Next::Wait => Driven::ended(),
        }
    }

    fn release(&self, hash: &str, stopped_at: Option<usize>) -> Result<Option<RunFile>, String> {
        match self
            .load()?
            .and_then(|file| stop_file(&file, hash, stopped_at))
        {
            Some(released) => self.save(&released).map(|()| Some(released)),
            None => Ok(None),
        }
    }

    fn log_notice(&self, notice: &str) {
        warn_if_err(
            Store::new(self.log.clone()).append(Role::User, &notice_record(notice)),
            "Failed to record the send-it notice",
        );
    }

    fn ended(&self, notice: &str, driving: &Driving) -> Driven {
        println!("{notice}");
        self.log_notice(notice);
        if let Err(error) = self.release(&driving.hash, None) {
            println!("{error}");
        }
        Driven::ended()
    }

    fn stopped(&self, reason: StopReason, driving: &Driving, monitor: &Monitor) -> Driven {
        let kept = self.release(&driving.hash, Some(driving.progress.step + 1));
        let notice = reason.notice(&driving.run, kept.as_ref().ok().and_then(Option::as_ref));
        println!("{notice}");
        self.log_notice(&notice);
        if let Err(error) = kept {
            println!("{error}");
        }
        if let Some(alert) = blocked_alert(&notice, &monitor.alert) {
            inbox_alert::deliver(&alert);
        }
        monitor.term.send(&blocked_report(&notice));
        Driven::ended()
    }

    pub(super) fn took_over(&self, active: &Active, monitor: &Monitor) {
        self.stopped(StopReason::TookOver, &active.driving, monitor);
    }

    pub(crate) fn off(&self) -> String {
        match fs::remove_file(&self.path) {
            Ok(()) => "send-it off: the run plan is deleted.".to_string(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => NO_RUN.to_string(),
            Err(error) => format!("Could not delete {}: {error}", self.path.display()),
        }
    }

    pub(crate) fn edit(&self, argv: &[String]) -> Screen {
        if !self.path.exists() {
            return Screen::text(NO_RUN);
        }
        let opened = open_in_editor(argv, &self.path);
        let screen = self.clear_approval();
        match opened {
            Ok(()) => screen,
            Err(error) => Screen {
                text: format!("Editor failed: {error}\n{}", screen.text),
                ..screen
            },
        }
    }

    pub(crate) fn save_template(&self, name: &str, ticket: Option<&str>) -> String {
        let saved = template_name(name).and_then(|name| {
            let file = self.load()?.ok_or(NO_RUN)?;
            let text = template_text(&file.plan, ticket)?;
            let path = self.templates.join(format!("{name}.toml"));
            let replaced = path.exists();
            fs::create_dir_all(&self.templates)
                .and_then(|()| fs::write(&path, &text))
                .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
            Ok(save_notice(name, ticket, &text, replaced))
        });
        saved.unwrap_or_else(|text| text)
    }

    fn template_names(&self) -> Result<Vec<String>, String> {
        let entries = match fs::read_dir(&self.templates) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(unreadable(&self.templates, error)),
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .filter_map(|path| Some(path.file_stem()?.to_str()?.to_string()))
            .collect();
        names.sort();
        Ok(names)
    }

    pub(crate) fn templates(&self) -> String {
        self.template_names()
            .map_or_else(|text| text, |names| render_templates(&names))
    }

    pub(crate) fn template_use(&self, text: &str) -> Option<(String, String)> {
        match_template(text, &self.template_names().ok()?)
    }

    pub(crate) fn use_template(&self, ticket: &str, name: &str) -> Screen {
        if let Some(refusal) = self.refusal() {
            return Screen::text(refusal);
        }
        let path = self.templates.join(format!("{name}.toml"));
        let stored = fs::read_to_string(&path)
            .map_err(|error| unreadable(&path, error))
            .and_then(|text| {
                fill_template(&text, ticket, self.limits)
                    .map_err(|error| format!("Template {name}: {error}"))
            })
            .and_then(|file| self.save(&file));
        match stored {
            Ok(()) => self.status(),
            Err(text) => Screen::text(text),
        }
    }

    fn clear_approval(&self) -> Screen {
        match self.load() {
            Ok(Some(file)) => match self.save(&RunFile {
                approved: None,
                ..file
            }) {
                Ok(()) => self.status(),
                Err(text) => Screen::text(text),
            },
            Ok(None) => Screen::text(NO_RUN),
            Err(text) => Screen::text(format!("{text}\nFix it with /send-it edit.")),
        }
    }
}

#[cfg(test)]
mod tests;
