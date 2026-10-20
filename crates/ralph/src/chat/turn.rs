use crate::backoff::{attempt_from, retry_on_outage, wait_unless, Attempt};
use crate::chat::memory::Store;
use crate::chat::prompt::screen_columns;
use crate::highlight::ThemeConfig;
use crate::invoke::build_chat_command;
use crate::signal;
use crate::spinner::{Spinner, SpinnerContext, SpinnerSessionInfo};
use crate::stream_processor::VerboseToolsConfig;
use crate::stream_processor::{extract_response_text, OutputBlock};
use crate::subprocess::{Session, SpinnerSubprocessConfig, SubprocessError};
use crate::warn::{warn, warn_if_err};
use crate::watches::locked;
use ralph_core::chat::{
    build_agents_args, build_attachments_args, build_nap_args, turn_prompt, turn_system_prompt,
    VIEW_LINES,
};
use ralph_core::chat_input::{turn_end, TurnEnd, TurnSignals};
use ralph_core::chat_memory::{
    frozen_block, reply_entries, tail_lines, trigger_entries, turn_prompt_bytes, usage_record,
    view_stats, CallKind, Frozen, Trigger, TurnParts,
};
use ralph_core::chat_skills::{expand_slash_command, ChatSkill};
use std::borrow::Cow;
use std::io::{self};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use termimad::MadSkin;

pub(super) fn rule(skin: &MadSkin, width: usize) -> String {
    skin.text("---", Some(width)).to_string()
}

pub(super) fn print_rule() {
    print!("{}", rule(&MadSkin::default(), screen_columns()));
}

pub(super) fn shown_output(blocks: &[OutputBlock]) -> bool {
    !blocks.is_empty()
}

pub(super) fn print_rule_after(blocks: &[OutputBlock]) {
    if shown_output(blocks) {
        print_rule();
    }
}

pub(super) fn spawn_nap(store: &Path) -> io::Result<()> {
    Command::new(std::env::current_exe()?)
        .args(build_nap_args(store))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
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

pub(super) struct Turn<'a> {
    pub(super) store: &'a Store,
    pub(super) store_dir: &'a Path,
    pub(super) mcp_args: &'a [String],
    pub(super) permission_mode: &'a str,
    pub(super) plugin_args: &'a [String],
    pub(super) skills: &'a [ChatSkill],
    pub(super) theme_config: &'a ThemeConfig,
    pub(super) frozen: Mutex<Option<Frozen>>,
    pub(super) frozen_window: usize,
    pub(super) recent_history_tokens: usize,
    pub(super) cancel: Arc<AtomicBool>,
}

impl Turn<'_> {
    pub(super) fn run(
        &self,
        trigger: Trigger,
        logged: &[String],
        message: &str,
    ) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
        self.cancel.store(false, Ordering::SeqCst);
        self.store.set_pending_turn(trigger)?;
        let _pending = PendingTurn(self.store);
        let (entries, tree) = self.store.load_settled(|| {
            let mut spinner = Spinner::with_session_context(0, chat_session_info());
            spinner.start_with_context(SpinnerContext::WaitingForMemory);
            spinner
        })?;
        let pinned = locked(&self.frozen).take();
        let frozen = frozen_block(&entries, &tree, VIEW_LINES, self.frozen_window, pinned);
        *locked(&self.frozen) = Some(frozen.clone());
        let tail = tail_lines(&entries, &tree, frozen.end, self.recent_history_tokens);
        if logged_before_reply(trigger) {
            self.append_logged(trigger, logged)?;
        }
        let sent = match trigger {
            Trigger::User => expand_slash_command(message, self.skills),
            Trigger::Worker => Cow::Borrowed(message),
        };
        let prompt = turn_prompt(&tail, &sent);
        let system = turn_system_prompt(&frozen.lines);
        let args = [
            self.mcp_args.to_vec(),
            build_attachments_args(self.store_dir),
            build_agents_args(),
            self.plugin_args.to_vec(),
        ]
        .concat();
        let prompt_bytes = turn_prompt_bytes(&TurnParts {
            system: &system,
            frozen: &frozen.lines,
            tail: &tail,
            message: &sent,
            prompt: &prompt,
            args: &args,
        });
        let command = build_chat_command(
            &system,
            &args,
            self.permission_mode,
            dirs::home_dir().as_deref(),
        );
        print_rule();
        let result = retry_on_outage(
            "chat turn",
            || {
                let result = Session::spawn(&SpinnerSubprocessConfig {
                    command: command.clone(),
                    timeout_secs: None,
                    theme_config: self.theme_config.clone(),
                    session_elapsed_ms: 0,
                    verbose_tools: VerboseToolsConfig::new(),
                    session_info: chat_session_info(),
                    pause: None,
                    cancel: Some(Arc::clone(&self.cancel)),
                })
                .and_then(|mut session| session.turn(&prompt));
                if self.stopped() {
                    Attempt::Done(result)
                } else {
                    attempt_from(result)
                }
            },
            |delay| wait_unless(|| self.stopped(), delay),
        );
        let end = turn_end(&TurnSignals {
            cancelled: self.cancel.swap(false, Ordering::SeqCst),
            terminated: signal::is_terminated(),
        });
        match end {
            TurnEnd::Exit => return Ok(TurnOutcome::Exit),
            TurnEnd::Cancelled => {
                if let Ok(partial) = &result {
                    print_rule_after(&partial.stream_result.output_blocks);
                }
                warn("Turn cancelled.");
                return Ok(TurnOutcome::Unanswered);
            }
            TurnEnd::Completed => {}
        }
        let result = match result {
            Ok(result) => result,
            Err(SubprocessError::Interrupted { .. }) => return Ok(TurnOutcome::Exit),
            Err(e) => {
                warn(format!("chat turn failed: {e}"));
                return Ok(TurnOutcome::Unanswered);
            }
        };
        if let Some(record) = usage_record(
            CallKind::Turn { trigger },
            chrono::Utc::now().to_rfc3339(),
            &result.stream_result.costs,
        )
        .map(|record| {
            record
                .with_view(view_stats(
                    &[frozen.lines.as_slice(), tail.as_slice()].concat(),
                ))
                .with_prompt(prompt_bytes)
        }) {
            warn_if_err(
                self.store.append_usage(&record),
                "Failed to record chat usage",
            );
        }
        print_rule();
        match extract_response_text(&result.stream_result.output_blocks) {
            Some(reply) if result.exit_code == 0 => {
                if !logged_before_reply(trigger) {
                    self.append_logged(trigger, logged)?;
                }
                reply_entries(&reply)
                    .iter()
                    .try_for_each(|(role, text)| self.store.append(*role, text))?;
                warn_if_err(spawn_nap(self.store_dir), "Failed to start chat memory nap");
                Ok(TurnOutcome::Replied)
            }
            _ => {
                warn(format!("chat turn failed (exit {})", result.exit_code));
                Ok(TurnOutcome::Unanswered)
            }
        }
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
