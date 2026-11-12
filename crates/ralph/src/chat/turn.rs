use crate::backoff::{attempt_from, retry_on_outage, wait_unless, Attempt};
use crate::chat::keepalive::Stamp;
use crate::chat::prompt::screen_columns;
use crate::chat::suggestion::SuggestionSlot;
use crate::chat::thread::{fork_view, Fork};
use crate::highlight::ThemeConfig;
use crate::invoke::{build_chat_command, with_chat_inbox, Isolation};
use crate::signal;
use crate::spinner::{Spinner, SpinnerContext, SpinnerSessionInfo};
use crate::stream_processor::VerboseToolsConfig;
use crate::stream_processor::{extract_response_text, OutputBlock};
use crate::subprocess::{
    recorded, OutputGate, Session, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use crate::ui_theme::UiColors;
use crate::warn::warning_line;
use ralph_core::chat::{
    build_agents_args, build_attachments_args, build_nap_args, build_tag_args, hit_blocking_limit,
    reset_reason, turn_prompt, turn_system_prompt, CustomPrompt, ReplyImages, ResetLimits,
    VIEW_LINES,
};
use ralph_core::chat_input::{turn_end, TurnEnd, TurnSignals};
use ralph_core::chat_local::{catch_up_prompt, unseen_lines};
use ralph_core::chat_memory::{
    carry_metadata, reply_entries, split_view, subagent_records, trigger_entries, turn_row,
    usage_record, view_stats, CallSite, Trigger, TurnTiming, ViewSplit,
};
use ralph_core::chat_process::{spend_over, ResetReason};
use ralph_core::chat_skills::{expand_slash_command, ChatSkill};
use ralph_core::sanitize::strip_controls;
use ralph_core::stream::IterationMetadata;
use ralph_server::chat::memory::{usage_path, NapWait, Store, NAP_WAIT_LIMIT};
use ralph_server::ledger;
use ralph_server::private_file::write_private;
use ralph_server::subprocess::spawn_detached;
use ralph_server::timing;
use ralph_server::watches::locked;
use ralph_server::workers::usage_store;
use std::borrow::Cow;
use std::io::{self};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::Instant;
use termimad::MadSkin;

pub(super) fn rule(skin: &MadSkin, width: usize) -> String {
    skin.text("---", Some(width)).to_string()
}

pub(super) fn rule_skin(ui: &UiColors) -> MadSkin {
    let mut skin = MadSkin::default();
    skin.horizontal_rule.set_fg(ui.rule);
    skin
}

pub(super) fn shown_output(blocks: &[OutputBlock]) -> bool {
    !blocks.is_empty()
}

pub(super) fn spawn_nap(store: &Path, run: &str) -> io::Result<()> {
    spawn_detached(
        Command::new(std::env::current_exe()?)
            .args(build_nap_args(store, run))
            .envs([ralph_core::worker::inbox_env(&std::fs::canonicalize(
                store,
            )?)]),
    )
}

pub(super) fn spawn_tag(store: &Path, run: &str) -> io::Result<()> {
    spawn_detached(
        Command::new(std::env::current_exe()?)
            .args(build_tag_args(store, run))
            .envs([ralph_core::worker::inbox_env(&std::fs::canonicalize(
                store,
            )?)]),
    )
}

pub(super) fn chat_session_info() -> SpinnerSessionInfo {
    SpinnerSessionInfo {
        persona: Some("chat".into()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnOutcome {
    Replied,
    Unanswered,
    Exit,
}

pub(super) fn logged_before_reply(trigger: Trigger) -> bool {
    trigger == Trigger::Worker
}

fn turn_ok(outcome: &Result<TurnOutcome, Box<dyn std::error::Error>>) -> bool {
    matches!(outcome, Ok(TurnOutcome::Replied))
}

pub(super) fn draft_after(outcome: TurnOutcome, message: String) -> Option<String> {
    (outcome == TurnOutcome::Unanswered).then_some(message)
}

/// Clears the in-flight turn marker on every exit path (reply, cancel, error).
struct PendingTurn<'a>(&'a Store);

impl Drop for PendingTurn<'_> {
    fn drop(&mut self) {
        self.0.clear_pending_turn();
    }
}

struct Prompts<'a> {
    sent: &'a str,
    live: &'a str,
}

struct Launch {
    config: SpinnerSubprocessConfig,
    split: ViewSplit,
    first_prompt: String,
    reset: Option<ResetReason>,
}

pub(super) struct Turn<'a> {
    pub(super) store: &'a Store,
    pub(super) store_dir: &'a Path,
    pub(super) fork: Option<&'a Fork>,
    pub(super) mcp_args: &'a [String],
    pub(super) system_prompt_file: &'a Path,
    pub(super) permission_mode: &'a str,
    pub(super) plugin_args: &'a [String],
    pub(super) skills: &'a [ChatSkill],
    pub(super) custom_prompt: &'a CustomPrompt,
    pub(super) reply_images: ReplyImages,
    pub(super) isolation: &'a Isolation,
    pub(super) theme_config: &'a ThemeConfig,
    pub(super) session: Arc<Mutex<Option<Session>>>,
    pub(super) process_turns: AtomicU64,
    pub(super) reset: Mutex<Option<ResetReason>>,
    pub(super) live: Arc<AtomicBool>,
    pub(super) reset_limits: ResetLimits,
    pub(super) recent_history_tokens: usize,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) gate: Arc<OutputGate>,
    pub(super) run: &'a str,
    pub(super) suggestions: SuggestionSlot,
    pub(super) last_turn_end: Stamp,
    pub(super) identity: Arc<Mutex<IterationMetadata>>,
    pub(super) last_cost_usd: Mutex<Option<f64>>,
    pub(super) timing: Mutex<TurnTiming>,
    pub(super) seen_len: Mutex<Option<usize>>,
}

impl Turn<'_> {
    fn print_rule(&self) {
        self.gate
            .write_out(&rule(&rule_skin(self.theme_config.ui()), screen_columns()));
    }

    fn print_rule_after(&self, blocks: &[OutputBlock]) {
        if shown_output(blocks) {
            self.print_rule();
        }
    }

    fn warn(&self, message: impl std::fmt::Display) {
        self.gate.write_err(&warning_line(message));
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

    pub(super) fn take_cost(&self) -> Option<f64> {
        locked(&self.last_cost_usd).take()
    }

    fn take_session(&self) -> Option<Session> {
        let held = match self.session.try_lock() {
            Ok(mut slot) => slot.take(),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner().take(),
            Err(TryLockError::WouldBlock) => {
                let mut spinner = Spinner::with_session_context(0, chat_session_info())
                    .with_color(self.theme_config.ui().spinner);
                spinner.start_with_context(SpinnerContext::WaitingForResponse);
                let taken = locked(&self.session).take();
                spinner.stop();
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

    pub(super) fn run(
        &self,
        trigger: Trigger,
        logged: &[String],
        message: &str,
    ) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
        let started = Instant::now();
        *locked(&self.timing) = TurnTiming::default();
        locked(&self.last_cost_usd).take();
        let outcome = self.run_turn(trigger, logged, message);
        let elapsed = started.elapsed();
        let turn_timing = *locked(&self.timing);
        timing::record(self.store_dir, Some(self.run), |key| {
            turn_row(key, elapsed, &turn_timing, turn_ok(&outcome))
        });
        *locked(&self.last_turn_end) = Some(chrono::Utc::now());
        self.live
            .store(locked(&self.session).is_some(), Ordering::SeqCst);
        outcome
    }

    fn run_turn(
        &self,
        trigger: Trigger,
        logged: &[String],
        message: &str,
    ) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
        self.cancel.store(false, Ordering::SeqCst);
        self.store.set_pending_turn(trigger)?;
        let _pending = PendingTurn(self.store);
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
            Trigger::User | Trigger::SendIt => expand_slash_command(message, self.skills),
            Trigger::Worker => Cow::Borrowed(message),
        };
        let live = catch_up_prompt(&unseen, &sent);
        let prompts = Prompts {
            sent: &sent,
            live: &live,
        };
        self.print_rule();
        let mut session = self.take_session();
        let early = turn_end(&TurnSignals {
            cancelled: self.cancel.swap(false, Ordering::SeqCst),
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
                    self.attempt(session.take(), pre_log_len, &prompts, &mut launch);
                session = live;
                ran_session = ran;
                if self.stopped() {
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
                let keep_waiting = wait_unless(|| self.stopped(), delay);
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
            cancelled: self.cancel.swap(false, Ordering::SeqCst),
            terminated: signal::is_terminated(),
        });
        if !keeps_session(&end) {
            self.drop_session(ResetReason::Restart);
        }
        let identity = {
            let mut held = locked(&self.identity);
            *held = carry_metadata(&held, launch.is_some(), &turn_metadata(&result));
            held.clone()
        };
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
                .with_process_turn(process_turns)
                .with_request_at(turn.request_at.map(|at| at.to_rfc3339()))
                .with_reset(launch.as_ref().and_then(|spawned| spawned.reset))
                .with_run(Some(self.run.to_string()))
                .with_store(Some(usage_store(self.store_dir)));
            self.warn_if_err(
                ledger::record(&usage_path(), &record),
                "Failed to record chat usage",
            );
            let subagents = subagent_records(&at, &turn.stream_result.costs, &identity)
                .into_iter()
                .map(|row| {
                    row.with_run(Some(self.run.to_string()))
                        .with_store(Some(usage_store(self.store_dir)))
                });
            self.warn_if_err(
                ledger::record_all(&usage_path(), subagents),
                "Failed to record subagent usage",
            );
        }
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
        *locked(&self.last_cost_usd) = costs
            .cost_usd
            .map(|total| spend_over(total, result.cost_before, process_turns == 1).unwrap_or(0.0));
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
                self.warn_if_err(
                    spawn_nap(self.store_dir, self.run),
                    "Failed to start chat memory nap",
                );
                self.warn_if_err(
                    spawn_tag(self.store_dir, self.run),
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
        prompts: &Prompts,
        launch: &mut Option<Launch>,
    ) -> (
        Option<Session>,
        Result<StreamingSubprocessResult, SubprocessError>,
        bool,
    ) {
        let was_reused = reused.is_some();
        let started = match reused {
            Some(live) => Ok(live),
            None => self.start(pre_log_len, prompts.sent, launch),
        };
        let (session, result, ran) = match started {
            Err(e) => (None, Err(e), false),
            Ok(mut live) => {
                let prompt = launch
                    .as_ref()
                    .map_or(prompts.live, |spawned| spawned.first_prompt.as_str());
                let result = live.turn(prompt);
                (Some(live), result, true)
            }
        };
        if relaunch(was_reused, &result) {
            drop(session);
            self.note_reset(ResetReason::Restart);
            return self.attempt(None, pre_log_len, prompts, launch);
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
            let mut spinner = Spinner::with_session_context(0, chat_session_info())
                .with_color(self.theme_config.ui().spinner);
            spinner.start_with_context(SpinnerContext::WaitingForMemory);
            spinner
        });
        self.note_timing(|held| held.with_memory_wait(waited_from.elapsed()));
        let (mut entries, tree, wait) = loaded?;
        if wait == NapWait::Capped {
            self.warn("chat memory is still napping; this session starts with older summaries");
        }
        if let Some(len) = pre_log_len {
            entries.truncate(len);
        }
        let split = split_view(&entries, &tree, VIEW_LINES, self.recent_history_tokens);
        let split = match self.fork {
            Some(fork) => fork_view(fork, split)?,
            None => split,
        };
        let first_prompt = turn_prompt(
            &[split.middle.as_slice(), split.tail.as_slice()].concat(),
            sent,
        );
        let system = turn_system_prompt(self.custom_prompt, self.reply_images, &split.older);
        write_system_prompt(self.system_prompt_file, &system)?;
        let args = [
            self.mcp_args.to_vec(),
            build_attachments_args(self.store_dir),
            build_agents_args(),
            self.plugin_args.to_vec(),
        ]
        .concat();
        let config = SpinnerSubprocessConfig {
            command: with_chat_inbox(
                build_chat_command(
                    self.system_prompt_file,
                    &args,
                    self.permission_mode,
                    dirs::home_dir().as_deref(),
                    self.isolation,
                ),
                self.store_dir,
            )?,
            timeout_secs: None,
            theme_config: self.theme_config.clone(),
            session_elapsed_ms: 0,
            verbose_tools: VerboseToolsConfig::new(),
            session_info: chat_session_info(),
            gate: Some(Arc::clone(&self.gate)),
            cancel: Some(Arc::clone(&self.cancel)),
            stdin: None,
        };
        Ok(Launch {
            config,
            split,
            first_prompt,
            reset: None,
        })
    }

    fn stopped(&self) -> bool {
        self.cancel.load(Ordering::SeqCst) || signal::is_terminated()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_processor::StreamProcessorResult;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    #[test]
    fn system_prompt_file_holds_the_old_argument_bytes_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system-prompt.md");
        let system = "rules\0 it's \"$HOME\"\n  ñ trailing \n";
        write_system_prompt(&path, &system.repeat(3)).unwrap();
        write_system_prompt(&path, system).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, b"rules it's \"$HOME\"\n  \xc3\xb1 trailing \n");
        assert_eq!(bytes, strip_controls(system).as_bytes());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    fn finished(session_id: Option<&str>, model: Option<&str>) -> StreamingSubprocessResult {
        StreamingSubprocessResult {
            exit_code: 0,
            stderr: String::new(),
            stream_result: StreamProcessorResult {
                metadata: IterationMetadata {
                    session_id: session_id.map(str::to_string),
                    model: model.map(str::to_string),
                },
                ..Default::default()
            },
            request_at: None,
            cost_before: None,
        }
    }

    #[test]
    fn turn_row_names_the_model_from_the_claude_init_line() {
        let mut processor = crate::stream_processor::StreamProcessor::new();
        [
            r#"{"type":"system","subtype":"init","session_id":"s1","tools":["Task","Bash","mcp__chat__help"],"mcp_servers":[{"name":"chat","status":"connected"}],"model":"claude-haiku-5-5","permissionMode":"default","slash_commands":["compact"],"agents":["general-purpose"]}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0018,"usage":{"input_tokens":3,"output_tokens":5},"session_id":"s1"}"#,
        ]
        .iter()
        .for_each(|line| {
            processor.process_line(line);
        });
        let result = Ok(StreamingSubprocessResult {
            exit_code: 0,
            stderr: String::new(),
            stream_result: processor.finish(),
            request_at: None,
            cost_before: None,
        });
        let identity = carry_metadata(&IterationMetadata::new(), true, &turn_metadata(&result));
        let costs = &result.as_ref().unwrap().stream_result.costs;
        let row = usage_record(
            CallSite::Turn {
                trigger: Trigger::User,
            },
            "t".into(),
            costs,
            &identity,
        )
        .unwrap();
        let row = serde_json::to_value(row).unwrap();
        assert_eq!(row["model"], "claude-haiku-5-5");
        assert_eq!(row["session_id"], "s1");
    }

    const PROBE_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0024732699999999996,"duration_ms":4404,"session_id":"47488cc1","usage":{"input_tokens":2,"cache_creation_input_tokens":10086,"cache_read_input_tokens":6887,"output_tokens":774,"output_tokens_details":{"thinking_tokens":565},"cache_creation":{"ephemeral_1h_input_tokens":10086,"ephemeral_5m_input_tokens":0}}}"#;

    fn probe_partial() -> StreamingSubprocessResult {
        let mut processor = crate::stream_processor::StreamProcessor::new();
        processor.process_line(PROBE_RESULT);
        StreamingSubprocessResult {
            exit_code: 1,
            stderr: String::new(),
            stream_result: processor.finish(),
            request_at: None,
            cost_before: None,
        }
    }

    fn recorded_cache_writes(
        result: &Result<StreamingSubprocessResult, SubprocessError>,
    ) -> Option<u64> {
        recorded(result)
            .and_then(|turn| turn.stream_result.costs.usage.as_ref())
            .and_then(|usage| usage.cache_creation_input_tokens)
    }

    #[test]
    fn a_cancelled_turn_with_a_result_still_records_its_usage() {
        let cancelled = Ok(probe_partial());
        assert_eq!(recorded_cache_writes(&cancelled), Some(10086));
    }

    #[test]
    fn an_interrupted_or_timed_out_turn_records_its_partial_usage() {
        let interrupted = Err(SubprocessError::Interrupted {
            partial_result: Box::new(probe_partial()),
        });
        assert_eq!(recorded_cache_writes(&interrupted), Some(10086));
        let timed_out = Err(timeout(probe_partial()));
        assert_eq!(recorded_cache_writes(&timed_out), Some(10086));
        let stale = Err(SubprocessError::Stale);
        assert_eq!(recorded_cache_writes(&stale), None);
    }

    fn previous() -> IterationMetadata {
        IterationMetadata {
            session_id: Some("old".to_string()),
            model: Some("old-model".to_string()),
        }
    }

    fn timeout(partial: StreamingSubprocessResult) -> SubprocessError {
        SubprocessError::Timeout {
            timeout_secs: 1,
            partial_result: Box::new(partial),
        }
    }

    #[test]
    fn turn_metadata_reads_a_finished_turn() {
        let result = Ok(finished(Some("s1"), Some("m1")));
        let meta = turn_metadata(&result);
        assert_eq!(meta.session_id.as_deref(), Some("s1"));
        assert_eq!(meta.model.as_deref(), Some("m1"));
    }

    #[test]
    fn turn_metadata_reads_a_timeout_partial() {
        let result = Err(timeout(finished(Some("s2"), None)));
        let meta = turn_metadata(&result);
        assert_eq!(meta.session_id.as_deref(), Some("s2"));
        assert_eq!(meta.model, None);
    }

    #[test]
    fn turn_metadata_reads_an_interrupted_partial() {
        let result = Err(SubprocessError::Interrupted {
            partial_result: Box::new(finished(Some("s3"), None)),
        });
        assert_eq!(turn_metadata(&result).session_id.as_deref(), Some("s3"));
    }

    #[test]
    fn turn_metadata_is_empty_for_stale() {
        let result = Err(SubprocessError::Stale);
        assert_eq!(turn_metadata(&result), IterationMetadata::new());
    }

    #[test]
    fn launched_stale_turn_clears_previous_identity() {
        let result = Err(SubprocessError::Stale);
        let carried = carry_metadata(&previous(), true, &turn_metadata(&result));
        assert_eq!(carried.session_id, None);
        assert_eq!(carried.model, None);
    }

    #[test]
    fn reused_failed_turn_keeps_previous_identity() {
        let result = Err(SubprocessError::Stale);
        let carried = carry_metadata(&previous(), false, &turn_metadata(&result));
        assert_eq!(carried.session_id.as_deref(), Some("old"));
        assert_eq!(carried.model.as_deref(), Some("old-model"));
    }

    #[test]
    fn launched_timeout_takes_partial_session_id() {
        let result = Err(timeout(finished(Some("new"), None)));
        let carried = carry_metadata(&previous(), true, &turn_metadata(&result));
        assert_eq!(carried.session_id.as_deref(), Some("new"));
        assert_eq!(carried.model, None);
    }

    #[test]
    fn process_turns_stay_when_no_session_ran() {
        assert_eq!(next_process_turns(false, false, 4), 4);
        assert_eq!(next_process_turns(false, true, 4), 4);
    }

    #[test]
    fn process_turns_restart_at_one_on_a_launch() {
        assert_eq!(next_process_turns(true, true, 4), 1);
    }

    #[test]
    fn process_turns_grow_on_a_reused_session() {
        assert_eq!(next_process_turns(true, false, 4), 5);
    }

    #[test]
    fn completed_turn_keeps_session() {
        assert!(keeps_session(&TurnEnd::Completed));
    }

    #[test]
    fn cancelled_turn_drops_session() {
        assert!(!keeps_session(&TurnEnd::Cancelled));
    }

    #[test]
    fn exit_keeps_session() {
        assert!(keeps_session(&TurnEnd::Exit));
    }

    #[test]
    fn reused_session_that_is_stale_relaunches() {
        assert!(relaunch(true, &Err::<(), _>(SubprocessError::Stale)));
    }

    #[test]
    fn fresh_session_that_is_stale_does_not_relaunch() {
        assert!(!relaunch(false, &Err::<(), _>(SubprocessError::Stale)));
    }

    #[test]
    fn reused_session_with_leftover_result_relaunches() {
        assert!(relaunch(
            true,
            &Err::<(), _>(SubprocessError::LeftoverResult)
        ));
    }

    #[test]
    fn fresh_session_with_leftover_result_does_not_relaunch() {
        assert!(!relaunch(
            false,
            &Err::<(), _>(SubprocessError::LeftoverResult)
        ));
    }

    #[test]
    fn reused_session_with_other_error_does_not_relaunch() {
        assert!(!relaunch(true, &Err::<(), _>(SubprocessError::Signaled)));
    }

    #[test]
    fn reused_session_that_succeeds_does_not_relaunch() {
        assert!(!relaunch(true, &Ok::<_, SubprocessError>(())));
    }

    #[test]
    fn stale_error_message_names_the_exit() {
        assert_eq!(
            SubprocessError::Stale.to_string(),
            "claude process exited before the turn"
        );
    }

    #[test]
    fn leftover_result_error_message_names_the_leftover() {
        assert_eq!(
            SubprocessError::LeftoverResult.to_string(),
            "claude process wrote a result after the last turn"
        );
    }

    #[test]
    fn only_a_replied_turn_is_timed_ok() {
        type Outcome = Result<TurnOutcome, Box<dyn std::error::Error>>;
        let ok = |outcome: Outcome| turn_ok(&outcome);
        assert!(ok(Ok(TurnOutcome::Replied)));
        assert!(!ok(Ok(TurnOutcome::Unanswered)));
        assert!(!ok(Ok(TurnOutcome::Exit)));
        assert!(!ok(Err("store".into())));
    }

    #[test]
    fn a_turn_row_for_a_failed_turn_keeps_the_waits_and_says_not_ok() {
        use ralph_core::chat_memory::{turn_row, TimingKey};
        let timing = TurnTiming::default()
            .with_memory_wait(Duration::from_millis(12))
            .with_outage_wait(Duration::from_millis(30));
        let key = TimingKey {
            store: "/s",
            run: Some("r"),
            at: chrono::Utc::now(),
        };
        let line = ralph_core::chat_memory::encode_timing(&turn_row(
            &key,
            Duration::from_millis(80),
            &timing,
            false,
        ))
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["memory_wait_ms"], 12);
        assert_eq!(value["outage_retries"], 1);
    }
}
