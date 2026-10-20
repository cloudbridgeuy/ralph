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
    reply_entries, split_view, trigger_entries, turn_prompt_bytes, usage_record, view_stats,
    CallKind, Trigger, TurnParts, ViewSplit,
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

struct Launch {
    config: SpinnerSubprocessConfig,
    split: ViewSplit,
    system: String,
    args: Vec<String>,
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
    pub(super) session: Mutex<Option<Session>>,
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
        if logged_before_reply(trigger) {
            self.append_logged(trigger, logged)?;
        }
        let sent = match trigger {
            Trigger::User => expand_slash_command(message, self.skills),
            Trigger::Worker => Cow::Borrowed(message),
        };
        let mut session = locked(&self.session).take().filter(Session::is_alive);
        let mut launch: Option<Launch> = None;
        print_rule();
        let result = retry_on_outage(
            "chat turn",
            || {
                launch = None;
                let result = match session.take() {
                    Some(live) => Ok(live),
                    None => self.launch(trigger, logged, &sent).and_then(|spawned| {
                        let live = Session::spawn(&spawned.config)?;
                        launch = Some(spawned);
                        Ok(live)
                    }),
                }
                .and_then(|mut live| {
                    let prompt = launch
                        .as_ref()
                        .map_or(sent.as_ref(), |spawned| spawned.first_prompt.as_str());
                    let result = live.turn(prompt);
                    session = Some(live);
                    result
                });
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
        *locked(&self.session) = session.filter(Session::is_alive);
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
            let Some(spawned) = &launch else {
                return record;
            };
            record
                .with_view(view_stats(
                    &[
                        spawned.split.older.as_slice(),
                        spawned.split.tail.as_slice(),
                    ]
                    .concat(),
                ))
                .with_prompt(turn_prompt_bytes(&TurnParts {
                    system: &spawned.system,
                    older: &spawned.split.older,
                    tail: &spawned.split.tail,
                    message: &sent,
                    prompt: &spawned.first_prompt,
                    args: &spawned.args,
                }))
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
                locked(&self.session).take();
                warn(format!("chat turn failed (exit {})", result.exit_code));
                Ok(TurnOutcome::Unanswered)
            }
        }
    }

    fn launch(
        &self,
        trigger: Trigger,
        logged: &[String],
        sent: &str,
    ) -> Result<Launch, SubprocessError> {
        let (mut entries, tree) = self.store.load_settled(|| {
            let mut spinner = Spinner::with_session_context(0, chat_session_info());
            spinner.start_with_context(SpinnerContext::WaitingForMemory);
            spinner
        })?;
        if logged_before_reply(trigger) {
            entries.truncate(entries.len().saturating_sub(logged.len()));
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
            pause: None,
            cancel: Some(Arc::clone(&self.cancel)),
        };
        Ok(Launch {
            config,
            split,
            system,
            args,
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
