use crate::backoff::{attempt_from, retry_on_outage, wait_unless, Attempt};
use crate::chat::memory::Store;
use crate::chat::prompt::screen_columns;
use crate::highlight::ThemeConfig;
use crate::invoke::build_chat_command;
use crate::signal;
use crate::spinner::{Spinner, SpinnerContext, SpinnerSessionInfo};
use crate::stream_processor::VerboseToolsConfig;
use crate::stream_processor::{extract_response_text, OutputBlock};
use crate::subprocess::{
    spawn_detached, OutputGate, Session, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use crate::warn::warning_line;
use crate::watches::locked;
use ralph_core::chat::{
    build_agents_args, build_attachments_args, build_nap_args, hit_blocking_limit, reset_due,
    turn_prompt, turn_system_prompt, ResetLimits, VIEW_LINES,
};
use ralph_core::chat_input::{turn_end, TurnEnd, TurnSignals};
use ralph_core::chat_memory::{
    reply_entries, split_view, trigger_entries, usage_record, view_stats, CallKind, Trigger,
    ViewSplit,
};
use ralph_core::chat_skills::{expand_slash_command, ChatSkill};
use std::borrow::Cow;
use std::io::{self};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use termimad::MadSkin;

pub(super) fn rule(skin: &MadSkin, width: usize) -> String {
    skin.text("---", Some(width)).to_string()
}

pub(super) fn shown_output(blocks: &[OutputBlock]) -> bool {
    !blocks.is_empty()
}

pub(super) fn spawn_nap(store: &Path) -> io::Result<()> {
    spawn_detached(Command::new(std::env::current_exe()?).args(build_nap_args(store)))
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

struct Launch {
    config: SpinnerSubprocessConfig,
    split: ViewSplit,
    first_prompt: String,
}

pub(super) struct Turn<'a> {
    pub(super) store: &'a Store,
    pub(super) store_dir: &'a Path,
    pub(super) mcp_args: &'a [String],
    pub(super) permission_mode: &'a str,
    pub(super) plugin_args: &'a [String],
    pub(super) skills: &'a [ChatSkill],
    pub(super) theme_config: &'a ThemeConfig,
    pub(super) session: Arc<Mutex<Option<Session>>>,
    pub(super) process_turns: AtomicU64,
    pub(super) reset_limits: ResetLimits,
    pub(super) recent_history_tokens: usize,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) gate: Arc<OutputGate>,
}

impl Turn<'_> {
    fn print_rule(&self) {
        self.gate
            .write_out(&rule(&MadSkin::default(), screen_columns()));
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

    pub(super) fn run(
        &self,
        trigger: Trigger,
        logged: &[String],
        message: &str,
    ) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
        self.cancel.store(false, Ordering::SeqCst);
        self.store.set_pending_turn(trigger)?;
        let _pending = PendingTurn(self.store);
        let pre_log_len = logged_before_reply(trigger)
            .then(|| self.store.log_len())
            .transpose()?;
        if logged_before_reply(trigger) {
            self.append_logged(trigger, logged)?;
        }
        let sent = match trigger {
            Trigger::User => expand_slash_command(message, self.skills),
            Trigger::Worker => Cow::Borrowed(message),
        };
        let mut session = locked(&self.session)
            .take()
            .and_then(|mut live| live.is_alive().then_some(live));
        let mut launch: Option<Launch> = None;
        self.print_rule();
        let result = retry_on_outage(
            "chat turn",
            || {
                launch = None;
                let (live, result) = self.attempt(session.take(), pre_log_len, &sent, &mut launch);
                session = live;
                if self.stopped() {
                    Attempt::Done(result)
                } else {
                    let attempt = attempt_from(result);
                    if matches!(attempt, Attempt::Failed(..)) {
                        session = None;
                    }
                    attempt
                }
            },
            |delay| wait_unless(|| self.stopped(), delay),
        );
        *locked(&self.session) = session.and_then(|mut live| live.is_alive().then_some(live));
        let process_turns = if launch.is_some() {
            1
        } else {
            self.process_turns.load(Ordering::SeqCst) + 1
        };
        self.process_turns.store(process_turns, Ordering::SeqCst);
        let end = turn_end(&TurnSignals {
            cancelled: self.cancel.swap(false, Ordering::SeqCst),
            terminated: signal::is_terminated(),
        });
        if !keeps_session(&end) {
            locked(&self.session).take();
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
        if let Some(record) = usage_record(
            CallKind::Turn { trigger },
            chrono::Utc::now().to_rfc3339(),
            &result.stream_result.costs,
        )
        .map(|record| {
            let Some(spawned) = &launch else {
                return record;
            };
            record.with_view(view_stats(
                &[
                    spawned.split.older.as_slice(),
                    spawned.split.tail.as_slice(),
                ]
                .concat(),
            ))
        }) {
            self.warn_if_err(
                self.store.append_usage(&record),
                "Failed to record chat usage",
            );
        }
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
                self.warn_if_err(spawn_nap(self.store_dir), "Failed to start chat memory nap");
                if reset_due(&self.reset_limits, process_turns, costs) {
                    locked(&self.session).take();
                }
                Ok(TurnOutcome::Replied)
            }
            _ => {
                locked(&self.session).take();
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
        sent: &str,
        launch: &mut Option<Launch>,
    ) -> (
        Option<Session>,
        Result<StreamingSubprocessResult, SubprocessError>,
    ) {
        let was_reused = reused.is_some();
        let started = match reused {
            Some(live) => Ok(live),
            None => self.start(pre_log_len, sent, launch),
        };
        let (session, result) = match started {
            Err(e) => (None, Err(e)),
            Ok(mut live) => {
                let prompt = launch
                    .as_ref()
                    .map_or(sent, |spawned| spawned.first_prompt.as_str());
                let result = live.turn(prompt);
                (Some(live), result)
            }
        };
        if relaunch(was_reused, &result) {
            drop(session);
            return self.attempt(None, pre_log_len, sent, launch);
        }
        (session, result)
    }

    fn start(
        &self,
        pre_log_len: Option<usize>,
        sent: &str,
        launch: &mut Option<Launch>,
    ) -> Result<Session, SubprocessError> {
        let spawned = self.launch(pre_log_len, sent)?;
        let live = Session::spawn(&spawned.config)?;
        *launch = Some(spawned);
        Ok(live)
    }

    fn launch(&self, pre_log_len: Option<usize>, sent: &str) -> Result<Launch, SubprocessError> {
        let (mut entries, tree) = self.store.load_after_nap(|| {
            let mut spinner = Spinner::with_session_context(0, chat_session_info());
            spinner.start_with_context(SpinnerContext::WaitingForMemory);
            spinner
        })?;
        if let Some(len) = pre_log_len {
            entries.truncate(len);
        }
        let split = split_view(&entries, &tree, VIEW_LINES, self.recent_history_tokens);
        let first_prompt = turn_prompt(&split.tail, sent);
        let system = turn_system_prompt(&split.older);
        let args = [
            self.mcp_args.to_vec(),
            build_attachments_args(self.store_dir),
            build_agents_args(),
            self.plugin_args.to_vec(),
        ]
        .concat();
        let config = SpinnerSubprocessConfig {
            command: build_chat_command(
                &system,
                &args,
                self.permission_mode,
                dirs::home_dir().as_deref(),
            ),
            timeout_secs: None,
            theme_config: self.theme_config.clone(),
            session_elapsed_ms: 0,
            verbose_tools: VerboseToolsConfig::new(),
            session_info: chat_session_info(),
            gate: Some(Arc::clone(&self.gate)),
            cancel: Some(Arc::clone(&self.cancel)),
        };
        Ok(Launch {
            config,
            split,
            first_prompt,
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

fn relaunch<T>(reused: bool, result: &Result<T, SubprocessError>) -> bool {
    reused
        && matches!(
            result,
            Err(SubprocessError::Stale | SubprocessError::LeftoverResult)
        )
}

fn keeps_session(end: &TurnEnd) -> bool {
    !matches!(end, TurnEnd::Cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
