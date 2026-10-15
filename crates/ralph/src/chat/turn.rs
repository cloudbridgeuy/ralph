use crate::chat::memory::Store;
use crate::chat::prompt::screen_columns;
use crate::highlight::ThemeConfig;
use crate::invoke::build_chat_command;
use crate::signal;
use crate::spinner::{Spinner, SpinnerContext, SpinnerSessionInfo};
use crate::stream_processor::VerboseToolsConfig;
use crate::stream_processor::{extract_response_text, OutputBlock};
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, SubprocessError,
};
use crate::warn::{warn, warn_if_err};
use crate::watches::locked;
use ralph_core::chat::{
    build_agents_args, build_attachments_args, build_mcp_args, build_nap_args, turn_prompt,
    turn_system_prompt, VIEW_LINES,
};
use ralph_core::chat_input::{turn_end, TurnEnd, TurnSignals};
use ralph_core::chat_memory::{
    frozen_block, tail_lines, usage_record, view_stats, CallKind, Frozen, Role, Trigger,
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

pub(super) fn spawn_nap(store: &Path, timeout_secs: u64) -> io::Result<()> {
    Command::new(std::env::current_exe()?)
        .args(build_nap_args(store, timeout_secs))
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

/// Clears the in-flight turn marker on every exit path (reply, cancel, timeout, error).
struct PendingTurn<'a>(&'a Store);

impl Drop for PendingTurn<'_> {
    fn drop(&mut self) {
        self.0.clear_pending_turn();
    }
}

pub(super) struct Turn<'a> {
    pub(super) store: &'a Store,
    pub(super) store_dir: &'a Path,
    pub(super) mcp_config_path: &'a Path,
    pub(super) permission_mode: &'a str,
    pub(super) plugin_args: &'a [String],
    pub(super) skills: &'a [ChatSkill],
    pub(super) timeout_secs: u64,
    pub(super) theme_config: &'a ThemeConfig,
    pub(super) frozen: Mutex<Option<Frozen>>,
    pub(super) frozen_window: usize,
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
        let tail = tail_lines(&entries, &tree, frozen.end);
        if logged_before_reply(trigger) {
            self.append_logged(trigger, logged)?;
        }
        let sent = match trigger {
            Trigger::User => expand_slash_command(message, self.skills),
            Trigger::Worker => Cow::Borrowed(message),
        };
        let command = build_chat_command(
            &turn_prompt(&tail, &sent),
            &turn_system_prompt(&frozen.lines),
            &[
                build_mcp_args(self.mcp_config_path),
                build_attachments_args(self.store_dir),
                build_agents_args(),
                self.plugin_args.to_vec(),
            ]
            .concat(),
            self.permission_mode,
            dirs::home_dir().as_deref(),
        );
        print_rule();
        let result = invoke_subprocess_with_spinner_config(&SpinnerSubprocessConfig {
            command,
            timeout_secs: self.timeout_secs,
            theme_config: self.theme_config.clone(),
            session_elapsed_ms: 0,
            verbose_tools: VerboseToolsConfig::new(),
            session_info: chat_session_info(),
            pause: None,
            cancel: Some(Arc::clone(&self.cancel)),
        });
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
            Err(SubprocessError::Timeout {
                timeout_secs,
                partial_result,
            }) => {
                print_rule_after(&partial_result.stream_result.output_blocks);
                warn(format!(
                    "chat turn timed out after {timeout_secs}s; restart with a larger --timeout for long work"
                ));
                return Ok(TurnOutcome::Unanswered);
            }
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
            record.with_view(view_stats(
                &[frozen.lines.as_slice(), tail.as_slice()].concat(),
            ))
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
                self.store.append(Role::Assistant, &reply)?;
                warn_if_err(
                    spawn_nap(self.store_dir, self.timeout_secs),
                    "Failed to start chat memory nap",
                );
                Ok(TurnOutcome::Replied)
            }
            _ => {
                warn(format!("chat turn failed (exit {})", result.exit_code));
                Ok(TurnOutcome::Unanswered)
            }
        }
    }

    fn append_logged(&self, trigger: Trigger, logged: &[String]) -> io::Result<()> {
        logged
            .iter()
            .try_for_each(|text| self.store.append(trigger.role(), text))
    }
}
