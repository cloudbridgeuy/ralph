use crate::backoff::{attempt_from, retry_on_outage, wait_unless, Attempt};
use crate::chat::fork::{fork_view, Fork};
use crate::chat::memory::{usage_path, NapWait, Store, NAP_WAIT_LIMIT};
use crate::chat::suggestion::SuggestionCell;
use crate::history_log;
use crate::invoke::{build_chat_command, with_chat_inbox, Isolation};
use crate::ledger;
use crate::private_file::write_private;
use crate::subprocess::spawn_detached;
use crate::subprocess::{
    recorded, OutputGate, Session, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use crate::timing;
use crate::watches::locked;
use crate::workers::usage_store;
use chrono::{DateTime, Utc};
use ralph_core::cancel::CancelToken;
use ralph_core::chat::{
    build_agents_args, build_attachments_args, build_nap_args, build_tag_args, hit_blocking_limit,
    reset_reason, turn_prompt, turn_system_prompt, CustomPrompt, ReplyImages, ResetLimits,
    VIEW_LINES,
};
use ralph_core::chat_input::{turn_end, TurnEnd, TurnSignals};
use ralph_core::chat_local::{catch_up_prompt, unseen_lines};
use ralph_core::chat_memory::{
    carry_metadata, reply_entries, resolve_scope, scope_label, split_view, subagent_records,
    trigger_entries, turn_row, usage_record, view_line_ids, view_stats, CallSite, Trigger,
    TurnTiming, ViewSplit,
};
use ralph_core::chat_process::{turn_cost, ResetReason};
use ralph_core::chat_skills::{expand_slash_command, ChatSkill};
use ralph_core::chat_suggestion::from_reply;
use ralph_core::chat_thread::Scope;
use ralph_core::sanitize::strip_controls;
use ralph_core::stream::blocks::{extract_response_text, OutputBlock, VerboseToolsConfig};
use ralph_core::stream::IterationMetadata;
use ralph_core::turn_event::{TurnEvent, TurnSink, Wait};
use std::borrow::Cow;
use std::ffi::OsString;
use std::io::{self};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::Instant;

fn shown_output(blocks: &[OutputBlock]) -> bool {
    blocks
        .iter()
        .any(|block| !matches!(block, OutputBlock::Separator))
}

fn background_command(
    exe: Option<&Path>,
    args: Vec<OsString>,
    store: &Path,
) -> io::Result<Command> {
    let exe =
        exe.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "ralph executable not found"))?;
    let mut command = Command::new(exe);
    command
        .args(args)
        .envs([ralph_core::worker::inbox_env(&std::fs::canonicalize(
            store,
        )?)]);
    Ok(command)
}

fn spawn_nap(exe: Option<&Path>, store: &Path, run: &str) -> io::Result<()> {
    spawn_detached(&mut background_command(
        exe,
        build_nap_args(store, run),
        store,
    )?)
}

fn spawn_tag(exe: Option<&Path>, store: &Path, run: &str) -> io::Result<()> {
    spawn_detached(&mut background_command(
        exe,
        build_tag_args(store, run),
        store,
    )?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Replied,
    Unanswered,
    Exit,
}

fn logged_before_reply(trigger: Trigger) -> bool {
    trigger == Trigger::Worker
}

fn turn_ok(outcome: &Result<TurnOutcome, Box<dyn std::error::Error>>) -> bool {
    matches!(outcome, Ok(TurnOutcome::Replied))
}

pub fn draft_after(outcome: TurnOutcome, message: String) -> Option<String> {
    (outcome == TurnOutcome::Unanswered).then_some(message)
}

/// Clears the in-flight turn marker on every exit path (reply, cancel, error).
struct PendingTurn<'a>(&'a Store);

impl Drop for PendingTurn<'_> {
    fn drop(&mut self) {
        self.0.clear_pending_turn();
    }
}

struct TurnInput<'a> {
    sent: &'a str,
    live: &'a str,
    cancel: &'a CancelToken,
}

struct TurnRequest<'a> {
    trigger: Trigger,
    logged: &'a [String],
    message: &'a str,
    cancel: &'a CancelToken,
}

struct Launch {
    config: SpinnerSubprocessConfig,
    split: ViewSplit,
    first_prompt: String,
    reset: Option<ResetReason>,
}

pub type Stamp = Arc<Mutex<Option<DateTime<Utc>>>>;

pub struct Turn {
    pub store: Store,
    pub store_dir: PathBuf,
    pub fork: Option<Fork>,
    pub mcp_args: Vec<String>,
    pub system_prompt_file: PathBuf,
    pub permission_mode: String,
    pub plugin_args: Vec<String>,
    pub model_args: Vec<String>,
    pub skills: Vec<ChatSkill>,
    pub custom_prompt: CustomPrompt,
    pub reply_images: ReplyImages,
    pub isolation: Isolation,
    pub session: Arc<Mutex<Option<Session>>>,
    pub process_turns: AtomicU64,
    pub reset: Mutex<Option<ResetReason>>,
    pub live: Arc<AtomicBool>,
    pub reset_limits: ResetLimits,
    pub recent_history_tokens: usize,
    pub gate: Arc<OutputGate>,
    pub run: String,
    pub suggestions: SuggestionCell,
    pub last_turn_end: Stamp,
    pub identity: Arc<Mutex<IterationMetadata>>,
    pub last_cost_usd: Mutex<Option<f64>>,
    pub timing: Mutex<TurnTiming>,
    pub seen_len: Mutex<Option<usize>>,
    pub sink: Arc<dyn TurnSink>,
    pub show_tools: bool,
    pub terminated: fn() -> bool,
    pub exe: Option<PathBuf>,
}

impl Turn {
    fn print_rule(&self) {
        self.sink.emit(TurnEvent::Rule);
    }

    fn print_rule_after(&self, blocks: &[OutputBlock]) {
        if shown_output(blocks) {
            self.print_rule();
        }
    }

    fn warn(&self, message: impl std::fmt::Display) {
        self.sink.emit(TurnEvent::Warning(message.to_string()));
    }

    fn warn_if_err<T, E: std::fmt::Display>(&self, result: Result<T, E>, context: &str) {
        if let Err(e) = result {
            self.warn(format!("{context}: {e}"));
        }
    }

    fn note_timing(&self, update: impl FnOnce(TurnTiming) -> TurnTiming) {
        let mut held = locked(&self.timing);
        *held = update(*held);
    }

    fn note_reset(&self, reason: ResetReason) {
        locked(&self.reset).get_or_insert(reason);
    }

    fn live_or_note(&self, held: Option<Session>) -> Option<Session> {
        let mut held = held?;
        if held.is_alive() {
            return Some(held);
        }
        self.note_reset(ResetReason::Restart);
        None
    }

    fn unseen(&self) -> io::Result<Vec<String>> {
        let seen = *locked(&self.seen_len);
        match seen {
            Some(seen) if seen < self.store.log_len()? => {
                Ok(unseen_lines(&self.store.load_log_quiet()?.0, Some(seen)))
            }
            _ => Ok(Vec::new()),
        }
    }

    pub fn take_cost(&self) -> Option<f64> {
        locked(&self.last_cost_usd).take()
    }

    fn take_session(&self) -> Option<Session> {
        let held = match self.session.try_lock() {
            Ok(mut slot) => slot.take(),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().take(),
            Err(TryLockError::WouldBlock) => {
                self.sink.emit(TurnEvent::Busy(Wait::Response));
                let taken = locked(&self.session).take();
                self.sink.emit(TurnEvent::Idle);
                taken
            }
        };
        self.live_or_note(held)
    }

    fn drop_session(&self, reason: ResetReason) {
        if locked(&self.session).take().is_some() {
            self.note_reset(reason);
        }
    }

    pub fn run(
        &self,
        trigger: Trigger,
        logged: &[String],
        message: &str,
        cancel: &CancelToken,
    ) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
        let started = Instant::now();
        let mark = history_log::begin_turn(&self.store_dir, &self.run);
        *locked(&self.timing) = TurnTiming::default();
        locked(&self.last_cost_usd).take();
        let mut view = Vec::new();
        let request = TurnRequest {
            trigger,
            logged,
            message,
            cancel,
        };
        let outcome = self.run_turn(&request, &mut view);
        let elapsed = started.elapsed();
        let turn_timing = *locked(&self.timing);
        timing::record(&self.store_dir, Some(&self.run), |key| {
            turn_row(key, elapsed, &turn_timing, turn_ok(&outcome))
        });
        let fork = self.fork.as_ref().map(|fork| &fork.name);
        history_log::end_turn(
            &self.store_dir,
            &self.run,
            &mark,
            &scope_label(&resolve_scope(&Scope::Own, fork)),
            view,
        );
        *locked(&self.last_turn_end) = Some(chrono::Utc::now());
        self.live
            .store(locked(&self.session).is_some(), Ordering::SeqCst);
        outcome
    }

    fn run_turn(
        &self,
        request: &TurnRequest,
        view: &mut Vec<String>,
    ) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
        let TurnRequest {
            trigger,
            logged,
            message,
            cancel,
        } = *request;
        self.store.set_pending_turn(trigger)?;
        let _pending = PendingTurn(&self.store);
        let unseen = self.unseen().unwrap_or_else(|e| {
            self.warn(format!("Failed to read the chat log for the catch-up: {e}"));
            Vec::new()
        });
        let pre_log_len = logged_before_reply(trigger)
            .then(|| self.store.log_len())
            .transpose()?;
        if logged_before_reply(trigger) {
            self.append_logged(trigger, logged)?;
        }
        let sent = match trigger {
            Trigger::User | Trigger::SendIt => expand_slash_command(message, &self.skills),
            Trigger::Worker => Cow::Borrowed(message),
        };
        let live = catch_up_prompt(&unseen, &sent);
        let input = TurnInput {
            sent: &sent,
            live: &live,
            cancel,
        };
        self.print_rule();
        let mut session = self.take_session();
        let early = turn_end(&TurnSignals {
            cancelled: cancel.is_cancelled(),
            terminated: false,
        });
        if early == TurnEnd::Cancelled {
            *locked(&self.session) = session;
            self.warn("Turn cancelled.");
            return Ok(TurnOutcome::Unanswered);
        }
        let mut launch: Option<Launch> = None;
        let mut ran_session = false;
        let result = retry_on_outage(
            "chat turn",
            || {
                launch = None;
                self.suggestions.clear();
                let (live, result, ran) =
                    self.attempt(session.take(), pre_log_len, &input, &mut launch);
                session = live;
                ran_session = ran;
                if stopped(cancel, self.terminated) {
                    Attempt::Done(result)
                } else {
                    let attempt = attempt_from(result);
                    if matches!(attempt, Attempt::Failed(..)) && session.take().is_some() {
                        self.note_reset(ResetReason::Restart);
                    }
                    attempt
                }
            },
            |delay| {
                let waited_from = Instant::now();
                let keep_waiting = wait_unless(|| stopped(cancel, self.terminated), delay);
                self.note_timing(|held| held.with_outage_wait(waited_from.elapsed()));
                keep_waiting
            },
        );
        *locked(&self.session) = self.live_or_note(session);
        let process_turns = next_process_turns(
            ran_session,
            launch.is_some(),
            self.process_turns.load(Ordering::SeqCst),
        );
        self.process_turns.store(process_turns, Ordering::SeqCst);
        let end = turn_end(&TurnSignals {
            cancelled: cancel.is_cancelled(),
            terminated: (self.terminated)(),
        });
        if !keeps_session(&end) {
            self.drop_session(ResetReason::Restart);
        }
        let identity = {
            let mut held = locked(&self.identity);
            *held = carry_metadata(&held, launch.is_some(), &turn_metadata(&result));
            held.clone()
        };
        if let Some(spawned) = &launch {
            *view = view_line_ids(
                &[
                    spawned.split.older.as_slice(),
                    spawned.split.middle.as_slice(),
                    spawned.split.tail.as_slice(),
                ]
                .concat(),
            );
        }
        let at = chrono::Utc::now().to_rfc3339();
        if let Some((turn, mut record)) = recorded(&result).and_then(|turn| {
            usage_record(
                CallSite::Turn { trigger },
                at.clone(),
                &turn.stream_result.costs,
                &identity,
            )
            .map(|record| (turn, record))
        }) {
            if let Some(spawned) = &launch {
                record = record.with_view(view_stats(
                    &[
                        spawned.split.older.as_slice(),
                        spawned.split.middle.as_slice(),
                        spawned.split.tail.as_slice(),
                    ]
                    .concat(),
                ));
            }
            self.note_timing(|held| held.with_model_ms(turn.stream_result.costs.duration_ms));
            let record = record
                .with_turn_cost(turn.logged_before)
                .with_process_turn(process_turns)
                .with_request_at(turn.request_at.map(|at| at.to_rfc3339()))
                .with_reset(launch.as_ref().and_then(|spawned| spawned.reset))
                .with_run(Some(self.run.to_string()))
                .with_store(Some(usage_store(&self.store_dir)));
            self.warn_if_err(
                ledger::record(&usage_path(), &record),
                "Failed to record chat usage",
            );
            let subagents = subagent_records(&at, &turn.stream_result.costs, &identity)
                .into_iter()
                .map(|row| {
                    row.with_run(Some(self.run.to_string()))
                        .with_store(Some(usage_store(&self.store_dir)))
                });
            self.warn_if_err(
                ledger::record_all(&usage_path(), subagents),
                "Failed to record subagent usage",
            );
        }
        *locked(&self.last_cost_usd) = recorded(&result).and_then(|turn| {
            let total = turn.stream_result.costs.cost_usd?;
            Some(turn_cost(total, turn.cost_before).map_or(0.0, |cost| cost.max(0.0)))
        });
        match end {
            TurnEnd::Exit => return Ok(TurnOutcome::Exit),
            TurnEnd::Cancelled => {
                if let Ok(partial) = &result {
                    self.print_rule_after(&partial.stream_result.output_blocks);
                }
                self.warn("Turn cancelled.");
                return Ok(TurnOutcome::Unanswered);
            }
            TurnEnd::Completed => {}
        }
        let result = match result {
            Ok(result) => result,
            Err(SubprocessError::Interrupted { .. }) => return Ok(TurnOutcome::Exit),
            Err(e) => {
                self.warn(format!("chat turn failed: {e}"));
                return Ok(TurnOutcome::Unanswered);
            }
        };
        self.print_rule();
        let costs = &result.stream_result.costs;
        let blocked = hit_blocking_limit(costs);
        match extract_response_text(&result.stream_result.output_blocks) {
            Some(reply) if result.exit_code == 0 && !blocked => {
                if !logged_before_reply(trigger) {
                    self.append_logged(trigger, logged)?;
                }
                reply_entries(&reply)
                    .iter()
                    .try_for_each(|(role, text)| self.store.append(*role, text))?;
                *locked(&self.seen_len) = self.store.log_len().ok();
                if let Some(suggestion) = from_reply(&reply) {
                    self.suggestions.set(suggestion);
                }
                self.warn_if_err(
                    spawn_nap(self.exe.as_deref(), &self.store_dir, &self.run),
                    "Failed to start chat memory nap",
                );
                self.warn_if_err(
                    spawn_tag(self.exe.as_deref(), &self.store_dir, &self.run),
                    "Failed to start chat entry tagging",
                );
                if let Some(reason) = reset_reason(&self.reset_limits, process_turns, costs) {
                    if locked(&self.session).take().is_some() {
                        *locked(&self.reset) = Some(reason);
                    }
                }
                Ok(TurnOutcome::Replied)
            }
            _ => {
                self.drop_session(if blocked {
                    ResetReason::Tokens
                } else {
                    ResetReason::Restart
                });
                if blocked {
                    self.warn("chat context is full (blocking_limit); restarting claude from the memory store on the next turn");
                } else {
                    self.warn(format!("chat turn failed (exit {})", result.exit_code));
                }
                Ok(TurnOutcome::Unanswered)
            }
        }
    }

    fn attempt(
        &self,
        reused: Option<Session>,
        pre_log_len: Option<usize>,
        input: &TurnInput,
        launch: &mut Option<Launch>,
    ) -> (
        Option<Session>,
        Result<StreamingSubprocessResult, SubprocessError>,
        bool,
    ) {
        let was_reused = reused.is_some();
        let started = match reused {
            Some(live) => Ok(live),
            None => self.start(pre_log_len, input.sent, launch),
        };
        let (session, result, ran) = match started {
            Err(e) => (None, Err(e), false),
            Ok(mut live) => {
                let prompt = launch
                    .as_ref()
                    .map_or(input.live, |spawned| spawned.first_prompt.as_str());
                let result = live.turn(prompt, input.cancel);
                (Some(live), result, true)
            }
        };
        if relaunch(was_reused, &result) {
            drop(session);
            self.note_reset(ResetReason::Restart);
            return self.attempt(None, pre_log_len, input, launch);
        }
        (session, result, ran)
    }

    fn start(
        &self,
        pre_log_len: Option<usize>,
        sent: &str,
        launch: &mut Option<Launch>,
    ) -> Result<Session, SubprocessError> {
        let mut spawned = self.launch(pre_log_len, sent)?;
        let live = Session::spawn(&spawned.config)?;
        spawned.reset = locked(&self.reset).take();
        *launch = Some(spawned);
        Ok(live)
    }

    fn launch(&self, pre_log_len: Option<usize>, sent: &str) -> Result<Launch, SubprocessError> {
        let waited_from = Instant::now();
        let loaded = self.store.load_after_nap(NAP_WAIT_LIMIT, || {
            self.sink.emit(TurnEvent::Busy(Wait::Memory));
        });
        self.sink.emit(TurnEvent::Idle);
        self.note_timing(|held| held.with_memory_wait(waited_from.elapsed()));
        let (mut entries, tree, wait) = loaded?;
        if wait == NapWait::Capped {
            self.warn("chat memory is still napping; this session starts with older summaries");
        }
        if let Some(len) = pre_log_len {
            entries.truncate(len);
        }
        let split = split_view(&entries, &tree, VIEW_LINES, self.recent_history_tokens);
        let split = match &self.fork {
            Some(fork) => fork_view(fork, split)?,
            None => split,
        };
        let first_prompt = turn_prompt(
            &[split.middle.as_slice(), split.tail.as_slice()].concat(),
            sent,
        );
        let system = turn_system_prompt(&self.custom_prompt, self.reply_images, &split.older);
        write_system_prompt(&self.system_prompt_file, &system)?;
        let args = [
            self.mcp_args.to_vec(),
            build_attachments_args(&self.store_dir),
            build_agents_args(),
            self.plugin_args.to_vec(),
            self.model_args.to_vec(),
        ]
        .concat();
        let config = SpinnerSubprocessConfig {
            command: with_chat_inbox(
                build_chat_command(
                    &self.system_prompt_file,
                    &args,
                    &self.permission_mode,
                    dirs::home_dir().as_deref(),
                    &self.isolation,
                ),
                &self.store_dir,
            )?,
            timeout_secs: None,
            verbose_tools: VerboseToolsConfig::new(),
            show_tools: self.show_tools,
            terminated: self.terminated,
            sink: Arc::clone(&self.sink),
            gate: Some(Arc::clone(&self.gate)),
            interrupt: None,
            stdin: None,
        };
        Ok(Launch {
            config,
            split,
            first_prompt,
            reset: None,
        })
    }

    fn append_logged(&self, trigger: Trigger, logged: &[String]) -> io::Result<()> {
        trigger_entries(trigger, logged)
            .iter()
            .try_for_each(|(role, text)| self.store.append(*role, text))
    }
}

fn write_system_prompt(path: &Path, system: &str) -> io::Result<PathBuf> {
    write_private(path.to_path_buf(), strip_controls(system).as_bytes())
}

fn relaunch<T>(reused: bool, result: &Result<T, SubprocessError>) -> bool {
    reused
        && matches!(
            result,
            Err(SubprocessError::Stale | SubprocessError::LeftoverResult)
        )
}

fn turn_metadata(result: &Result<StreamingSubprocessResult, SubprocessError>) -> IterationMetadata {
    recorded(result).map_or_else(IterationMetadata::new, |turn| {
        turn.stream_result.metadata.clone()
    })
}

fn next_process_turns(ran: bool, launched: bool, current: u64) -> u64 {
    match (ran, launched) {
        (false, _) => current,
        (true, true) => 1,
        (true, false) => current + 1,
    }
}

fn keeps_session(end: &TurnEnd) -> bool {
    !matches!(end, TurnEnd::Cancelled)
}

fn stopped(cancel: &CancelToken, terminated: fn() -> bool) -> bool {
    cancel.is_cancelled() || terminated()
}

#[cfg(test)]
mod tests;
