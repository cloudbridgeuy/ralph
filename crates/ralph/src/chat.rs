use crate::ansi::{DIM, RESET};
use crate::chat_cell;
use crate::chat_inbox::{self, Display};
use crate::chat_memory::Store;
use crate::chat_route::{run_local, LocalRoute};
use crate::cli::ChatArgs;
use crate::clipboard::{paste_image, PASTE_IMAGE};
use crate::config::AppConfig;
use crate::highlight::{Highlighter, ThemeConfig};
use crate::inbox_alert;
use crate::invoke::build_chat_command;
use crate::markdown::{MarkdownTheme, ThemeRole};
use crate::render::{cap_wrap_width, render_markdown, wrap_width, RenderContext};
use crate::signal;
use crate::spinner::{Spinner, SpinnerContext, SpinnerSessionInfo};
use crate::stream_processor::VerboseToolsConfig;
use crate::stream_processor::{extract_response_text, OutputBlock};
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, SubprocessError,
};
use crate::warn::{warn, warn_if_err};
use crate::watch_requests;
use crate::watches::{locked, Watches};
use crate::worker_mcp::Inbox;
use crate::workers::{canonical_store, capture_pane, opencode_active, tmux_listing, Workers};
use ralph_core::chat::{
    build_agents_args, build_attachments_args, build_mcp_args, build_nap_args, mcp_config,
    resolve_frozen_window, resolve_worker_check_secs, resolve_wrap_width, select_servers,
    turn_prompt, turn_system_prompt, ChatMcpOptions, VIEW_LINES,
};
use ralph_core::chat_cell::resolve_cell_limits;
use ralph_core::chat_input::{input_lost_at_prompt, turn_end, InputWatch, TurnEnd, TurnSignals};
use ralph_core::chat_memory::{
    frozen_block, tail_lines, usage_record, view_stats, CallKind, Frozen, Role, Trigger,
};
use ralph_core::chat_skills::{
    build_plugin_args, expand_slash_command, plan_links, plugin_manifest, resolve_skills,
    ChatSkill, SkillSources,
};
use ralph_core::chat_status::{
    footer, last_turn_cache, latest_states, memory_size, orphan_hint, route_input, status_line,
    store_workers, watch_states, worker_states, Memory, Route, RunningWorker, Status,
};
use ralph_core::inbox::Selection;
use ralph_core::inbox_alert::{fresh_reports, inbox_alert, AlertConfig};
use ralph_core::worker::{
    blocked_path, ready_path, unreported_path, waiting_path, Report, ATTACHMENTS_DIR, INBOX_DIR,
    READY_DIR,
};
use ralph_core::worker_check::{
    check_every_polls, last_reports, plan, reported_since_nudge, CheckAction, CheckMemory,
    WorkerObservation, CHECK_STATUS, NUDGE_TEXT,
};
use reedline::{
    default_emacs_keybindings, EditCommand, EditMode, Emacs, FileBackedHistory, KeyCode,
    KeyModifiers, Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline,
    ReedlineEvent, ReedlineRawEvent, Signal, DEFAULT_MULTILINE_INDICATOR,
};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use termimad::MadSkin;

const PROMPT: &str = ">>> ";
const HISTORY_FILE: &str = "history.txt";
const EDITOR_FILE: &str = "editor.md";
const HISTORY_CAPACITY: usize = 1000;
const STORE_ENV: &str = "RALPH_CHAT_STORE";
const SKILLS_ENV: &str = "RALPH_CHAT_SKILLS";
const WRAP_WIDTH_ENV: &str = "RALPH_WRAP_WIDTH";
const CELL_OUTPUT_LIMIT_ENV: &str = "RALPH_CELL_OUTPUT_LIMIT";
const CELL_OUTPUT_BYTES_ENV: &str = "RALPH_CELL_OUTPUT_BYTES";
const POLL: Duration = Duration::from_millis(500);
const MEMORY_EVERY_POLLS: u64 = 10;
const DEFAULT_TERMINAL_WIDTH: usize = 80;
const INPUT_LOST_EXIT: i32 = 129;

fn resolve_store(
    flag: Option<PathBuf>,
    env: Option<PathBuf>,
    cwd: &Path,
    home: Option<&Path>,
) -> io::Result<PathBuf> {
    let env = env.filter(|dir| !dir.as_os_str().is_empty());
    match flag.or(env) {
        Some(dir) => Ok(cwd.join(dir)),
        None => home
            .map(|home| home.join(".ralph").join("chat"))
            .ok_or_else(|| io::Error::other("home directory not found")),
    }
}

struct ChatEditMode {
    emacs: Emacs,
    esc_pending: bool,
}

fn esc_esc(esc_pending: bool, event: ReedlineEvent) -> (bool, ReedlineEvent) {
    match (esc_pending, event) {
        (true, ReedlineEvent::Esc) => (false, ReedlineEvent::Edit(vec![EditCommand::Clear])),
        (false, ReedlineEvent::Esc) => (true, ReedlineEvent::Esc),
        (_, event) => (false, event),
    }
}

impl EditMode for ChatEditMode {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        let (esc_pending, event) = esc_esc(self.esc_pending, self.emacs.parse_event(event));
        self.esc_pending = esc_pending;
        event
    }

    fn edit_mode(&self) -> PromptEditMode {
        self.emacs.edit_mode()
    }
}

fn build_editor(store_dir: &Path) -> Result<Reedline, Box<dyn std::error::Error>> {
    let history = FileBackedHistory::with_file(HISTORY_CAPACITY, store_dir.join(HISTORY_FILE))?;
    let mut keybindings = default_emacs_keybindings();
    for (modifier, key_code) in [
        (KeyModifiers::ALT, KeyCode::Enter),
        (KeyModifiers::CONTROL, KeyCode::Char('j')),
    ] {
        keybindings.add_binding(
            modifier,
            key_code,
            ReedlineEvent::Edit(vec![EditCommand::InsertNewline]),
        );
    }
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('v'),
        ReedlineEvent::ExecuteHostCommand(PASTE_IMAGE.to_string()),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('g'),
        ReedlineEvent::OpenEditor,
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('o'),
        ReedlineEvent::Edit(vec![EditCommand::Redo]),
    );
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    Ok(Reedline::create()
        .with_history(Box::new(history))
        .with_edit_mode(Box::new(ChatEditMode {
            emacs: Emacs::new(keybindings),
            esc_pending: false,
        }))
        .with_buffer_editor(Command::new(editor), store_dir.join(EDITOR_FILE))
        .use_bracketed_paste(true))
}

fn running_workers(store_dir: &Path) -> Vec<RunningWorker> {
    store_workers(&tmux_listing(), &store_dir.to_string_lossy())
}

fn ready_ids(store_dir: &Path) -> Vec<String> {
    std::fs::read_dir(store_dir.join(READY_DIR))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Default)]
struct CheckState {
    memory: CheckMemory,
    panes: HashMap<String, String>,
    nudges: HashMap<String, Option<String>>,
    reports: HashMap<String, String>,
}

struct Monitor {
    inbox: Arc<Inbox>,
    watches: Watches,
    store: Store,
    store_dir: PathBuf,
    status: Mutex<Status>,
    seen: Mutex<HashMap<String, String>>,
    active: Mutex<Option<HashSet<String>>>,
    frozen_window: usize,
    worker_check_secs: Option<u64>,
    workers: Workers,
    check: Mutex<CheckState>,
    known: Mutex<Vec<Report>>,
    alert: AlertConfig,
}

impl Monitor {
    fn status(&self) -> Status {
        locked(&self.status).clone()
    }

    fn drain(&self, selection: Selection) -> io::Result<Vec<Report>> {
        let reports = self.inbox.drain(selection)?;
        locked(&self.seen).extend(latest_states(&reports));
        locked(&self.check).reports.extend(last_reports(&reports));
        Ok(reports)
    }

    fn observe(&self, state: &mut CheckState, worker: &RunningWorker) -> WorkerObservation {
        let id = &worker.id;
        let ready = ready_path(&self.store_dir, id).exists();
        let blocked = blocked_path(&self.store_dir, id).exists();
        let waiting = waiting_path(&self.store_dir, id).exists();
        let unreported = unreported_path(&self.store_dir, id).exists();
        let reported = reported_since_nudge(state.reports.get(id), state.nudges.get(id));
        let pane = (!(blocked || waiting || (ready && reported)))
            .then(|| capture_pane(&worker.session))
            .flatten();
        let pane_changed = match &pane {
            Some(pane) => state.panes.insert(id.clone(), pane.clone()).as_ref() != Some(pane),
            None => {
                state.panes.remove(id);
                true
            }
        };
        WorkerObservation {
            id: id.clone(),
            session: worker.session.clone(),
            ready,
            blocked,
            waiting,
            unreported,
            reported_since_nudge: reported,
            pane,
            pane_changed,
        }
    }

    fn check(&self) {
        let running = running_workers(&self.store_dir);
        let actions = {
            let mut state = locked(&self.check);
            let observations: Vec<WorkerObservation> = running
                .iter()
                .map(|worker| self.observe(&mut state, worker))
                .collect();
            state
                .panes
                .retain(|id, _| running.iter().any(|worker| &worker.id == id));
            state
                .nudges
                .retain(|id, _| running.iter().any(|worker| &worker.id == id));
            let (actions, memory) = plan(&state.memory, &observations);
            state.memory = memory;
            for action in &actions {
                if let CheckAction::Nudge(id) = action {
                    let last = state.reports.get(id).cloned();
                    state.nudges.insert(id.clone(), last);
                }
            }
            actions
        };
        for action in actions {
            match action {
                CheckAction::Nudge(id) => warn_if_err(
                    self.workers.reply(&id, NUDGE_TEXT),
                    format!("nudging worker {id}"),
                ),
                CheckAction::Inbox(id, text) => {
                    let report = Report {
                        worker: id.clone(),
                        at: chrono::Utc::now().to_rfc3339(),
                        status: CHECK_STATUS.to_string(),
                        text,
                    };
                    warn_if_err(
                        self.inbox.post(&report, 0),
                        format!("posting check for worker {id}"),
                    );
                }
            }
        }
    }

    fn measure_memory(&self) -> Option<Memory> {
        let (entries, tree) = self.store.load().ok()?;
        let frozen = frozen_block(&entries, &tree, VIEW_LINES, self.frozen_window, None);
        let tail = tail_lines(&entries, &tree, frozen.end);
        Some(memory_size(entries.len(), &[frozen.lines, tail].concat()))
    }

    fn alert_fresh(&self, pending: &[Report]) {
        let mut known = locked(&self.known);
        let fresh = fresh_reports(&known, pending);
        if !fresh.is_empty() && self.alert.enabled() {
            if let Some(alert) = inbox_alert(&fresh, inbox_alert::focus(), &self.alert) {
                inbox_alert::deliver(&alert);
            }
        }
        *known = pending.to_vec();
    }

    fn refresh(&self, with_memory: bool) -> bool {
        let peeked = self.inbox.peek();
        if let Ok(pending) = &peeked {
            self.alert_fresh(pending);
        }
        let pending = peeked.unwrap_or_default();
        locked(&self.check).reports.extend(last_reports(&pending));
        let seen = {
            let mut seen = locked(&self.seen);
            seen.extend(latest_states(&pending));
            seen.clone()
        };
        let running = running_workers(&self.store_dir);
        let active = {
            let mut active = locked(&self.active);
            if with_memory {
                *active = running
                    .iter()
                    .any(|worker| worker.opencode)
                    .then(opencode_active)
                    .flatten();
            }
            active.clone()
        };
        let workers = worker_states(&running, &seen, active.as_ref());
        let memory = if with_memory {
            self.measure_memory()
        } else {
            None
        };
        let turn = if with_memory {
            self.store
                .load_usage()
                .ok()
                .map(|text| last_turn_cache(&text))
        } else {
            None
        };
        let mut status = locked(&self.status);
        let next = Status {
            unread: pending.len(),
            workers,
            watches: watch_states(&self.watches.list(), chrono::Utc::now()),
            memory: memory.unwrap_or(status.memory),
            turn: turn.unwrap_or(status.turn),
        };
        let changed = *status != next;
        *status = next;
        changed
    }
}

fn watch_status(monitor: Arc<Monitor>, repaint: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let check_every = monitor
            .worker_check_secs
            .map(|secs| check_every_polls(secs, POLL));
        let mut tick: u64 = 0;
        loop {
            std::thread::sleep(POLL);
            tick += 1;
            watch_requests::start_queued(&monitor.watches, &monitor.inbox, &monitor.store_dir);
            if monitor.refresh(tick.is_multiple_of(MEMORY_EVERY_POLLS)) {
                repaint();
            }
            if check_every.is_some_and(|every| tick.is_multiple_of(every)) {
                monitor.check();
            }
        }
    });
}

fn stdin_hung_up() -> bool {
    let mut stdin = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: ralph's direct dependencies have no safe poll wrapper, and this is
    // the backup check for terminals that close without sending SIGHUP.
    // `stdin` is a valid, initialized pollfd on the stack and nfds is 1, so poll
    // only touches that one struct. fd 0 is not owned or closed here. A zero
    // timeout means poll never blocks. revents is read only when poll returns > 0.
    unsafe { libc::poll(&mut stdin, 1, 0) > 0 && stdin.revents & libc::POLLHUP != 0 }
}

fn watch_input(at_prompt: Arc<AtomicBool>, plugin_dir: Option<PathBuf>, watches: Watches) {
    let started_on_terminal = io::stdin().is_terminal();
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL);
        if input_lost_at_prompt(&InputWatch {
            hangup_signal: signal::is_terminated(),
            started_on_terminal,
            stdin_hung_up: stdin_hung_up(),
            at_prompt: at_prompt.load(Ordering::SeqCst),
        }) {
            if let Some(dir) = &plugin_dir {
                std::fs::remove_dir_all(dir).ok();
            }
            watches.kill_all();
            std::process::exit(INPUT_LOST_EXIT);
        }
    });
}

struct ChatPrompt {
    monitor: Arc<Monitor>,
    color: bool,
    notice: RefCell<Option<String>>,
}

impl ChatPrompt {
    fn dim(&self, line: &str) -> String {
        if self.color {
            format!("{DIM}{line}{RESET}\n")
        } else {
            format!("{line}\n")
        }
    }
}

pub(crate) fn screen_columns() -> usize {
    wrap_width(
        crossterm::terminal::size()
            .map(|(w, _)| usize::from(w))
            .unwrap_or(DEFAULT_TERMINAL_WIDTH),
    )
}

impl Prompt for ChatPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Owned(self.dim(&status_line(&self.monitor.status(), screen_columns())))
    }

    fn render_prompt_footer(&self) -> Cow<'_, str> {
        if let Some(notice) = self.notice.borrow().as_deref() {
            return Cow::Owned(self.dim(notice));
        }
        let hints = footer(self.monitor.status().unread, screen_columns());
        Cow::Owned(self.dim(hints.trim_end_matches('\n')))
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _edit_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed(PROMPT)
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed(DEFAULT_MULTILINE_INDICATOR)
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({prefix}reverse-search: {}) ",
            history_search.term
        ))
    }
}

fn next_route(editor: &mut Reedline, prompt: &ChatPrompt) -> io::Result<Option<Route>> {
    let attachments = prompt.monitor.store_dir.join(ATTACHMENTS_DIR);
    loop {
        match editor.read_line(prompt)? {
            Signal::Success(line) => {
                prompt.notice.take();
                return Ok(Some(route_input(&line, prompt.monitor.status().unread)));
            }
            Signal::HostCommand(command) if command == PASTE_IMAGE => {
                *prompt.notice.borrow_mut() = paste_image(editor, &attachments)
            }
            _ => return Ok(None),
        }
    }
}

fn rule(skin: &MadSkin, width: usize) -> String {
    skin.text("---", Some(width)).to_string()
}

fn print_rule() {
    print!("{}", rule(&MadSkin::default(), screen_columns()));
}

fn shown_output(blocks: &[OutputBlock]) -> bool {
    !blocks.is_empty()
}

fn print_rule_after(blocks: &[OutputBlock]) {
    if shown_output(blocks) {
        print_rule();
    }
}

fn spawn_nap(store: &Path, timeout_secs: u64) -> io::Result<()> {
    Command::new(std::env::current_exe()?)
        .args(build_nap_args(store, timeout_secs))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

fn chat_session_info() -> SpinnerSessionInfo {
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

fn logged_before_reply(trigger: Trigger) -> bool {
    trigger == Trigger::Worker
}

fn draft_after(outcome: TurnOutcome, message: String) -> Option<String> {
    (outcome == TurnOutcome::Unanswered).then_some(message)
}

/// Clears the in-flight turn marker on every exit path (reply, cancel, timeout, error).
struct PendingTurn<'a>(&'a Store);

impl Drop for PendingTurn<'_> {
    fn drop(&mut self) {
        self.0.clear_pending_turn();
    }
}

struct Turn<'a> {
    store: &'a Store,
    store_dir: &'a Path,
    mcp_config_path: &'a Path,
    permission_mode: &'a str,
    plugin_args: &'a [String],
    skills: &'a [ChatSkill],
    timeout_secs: u64,
    theme_config: &'a ThemeConfig,
    frozen: Mutex<Option<Frozen>>,
    frozen_window: usize,
    cancel: Arc<AtomicBool>,
}

impl Turn<'_> {
    fn run(
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

fn write_mcp_config(
    store: &Store,
    store_dir: &Path,
    exe: &Path,
    server_names: &[String],
    options: &ChatMcpOptions,
) -> io::Result<PathBuf> {
    let claude_config = read_claude_config();
    let imported = select_servers(&claude_config, server_names);
    for name in &imported.missing {
        warn(format!(
            "chat: MCP server \"{name}\" not found in ~/.claude.json; continuing without it"
        ));
    }
    store.write_mcp_config(&mcp_config(exe, store_dir, options, &imported.servers))
}

fn env_skills() -> Vec<PathBuf> {
    std::env::var_os(SKILLS_ENV)
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn usable_skills(dirs: Vec<PathBuf>) -> Vec<(PathBuf, String)> {
    dirs.into_iter()
        .filter_map(|dir| {
            let dir = std::path::absolute(&dir).unwrap_or(dir);
            match std::fs::read_to_string(dir.join("SKILL.md")) {
                Ok(skill_md) => Some((dir, skill_md)),
                Err(e) => {
                    warn(format!(
                        "chat: skipping skill {}: cannot read SKILL.md ({e})",
                        dir.display()
                    ));
                    None
                }
            }
        })
        .collect()
}

fn link_skills(found: Vec<(PathBuf, String)>) -> Vec<ChatSkill> {
    let dirs: Vec<PathBuf> = found.iter().map(|(dir, _)| dir.clone()).collect();
    let plan = plan_links(&dirs);
    for dir in &plan.dropped {
        warn(format!(
            "chat: skipping skill {}: name already taken by an earlier skill",
            dir.display()
        ));
    }
    plan.links
        .into_iter()
        .filter_map(|(name, dir)| {
            let skill_md = found
                .iter()
                .find(|(found_dir, _)| *found_dir == dir)?
                .1
                .clone();
            Some(ChatSkill {
                name,
                dir,
                skill_md,
            })
        })
        .collect()
}

fn write_skill_plugin(skills: &[ChatSkill]) -> io::Result<Option<tempfile::TempDir>> {
    if skills.is_empty() {
        return Ok(None);
    }
    let plugin = tempfile::tempdir()?;
    let manifest_dir = plugin.path().join(".claude-plugin");
    let skills_dir = plugin.path().join("skills");
    std::fs::create_dir_all(&manifest_dir)?;
    std::fs::create_dir_all(&skills_dir)?;
    std::fs::write(
        manifest_dir.join("plugin.json"),
        plugin_manifest().to_string(),
    )?;
    for skill in skills {
        std::os::unix::fs::symlink(&skill.dir, skills_dir.join(&skill.name))?;
    }
    Ok(Some(plugin))
}

fn read_claude_config() -> serde_json::Value {
    let Some(path) = dirs::home_dir().map(|home| home.join(".claude.json")) else {
        return serde_json::json!({});
    };
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            warn(format!("Failed to parse {}: {e}", path.display()));
            serde_json::json!({})
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => {
            warn(format!("Failed to read {}: {e}", path.display()));
            serde_json::json!({})
        }
    }
}

pub fn run(args: ChatArgs) -> Result<(), Box<dyn std::error::Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");
    let store_dir = resolve_store(
        args.store,
        std::env::var_os(STORE_ENV).map(PathBuf::from),
        &std::env::current_dir()?,
        dirs::home_dir().as_deref(),
    )?;
    std::fs::create_dir_all(store_dir.join(ATTACHMENTS_DIR))?;
    let store = Store::new(store_dir.clone());
    let exe = std::env::current_exe()?;
    let config = AppConfig::load().unwrap_or_else(|e| {
        warn(format!("Failed to load config: {e}"));
        AppConfig::default()
    });
    let permission_mode = config.chat_permission_mode(
        args.permission_mode.as_deref(),
        std::env::var(crate::invoke::CHAT_PERMISSION_MODE_ENV)
            .ok()
            .as_deref(),
    );
    let chat_config = config.chat;
    let frozen_window = resolve_frozen_window(args.frozen_window, chat_config.frozen_window)?;
    let worker_check_secs = resolve_worker_check_secs(chat_config.worker_check_secs);
    let wrap_cap = resolve_wrap_width(
        args.wrap_width,
        std::env::var(WRAP_WIDTH_ENV).ok().as_deref(),
        chat_config.wrap_width,
    )?;
    cap_wrap_width(wrap_cap);
    let cell_limits = resolve_cell_limits(
        std::env::var(CELL_OUTPUT_LIMIT_ENV).ok().as_deref(),
        std::env::var(CELL_OUTPUT_BYTES_ENV).ok().as_deref(),
        chat_config.cell_output_limit,
        chat_config.cell_output_bytes,
    )?;
    let mcp_config_path = write_mcp_config(
        &store,
        &store_dir,
        &exe,
        &chat_config.mcp_servers,
        &ChatMcpOptions {
            permission_mode: &permission_mode,
            worktree_dir: args.worktree_dir.as_deref(),
        },
    )?;
    let skills = link_skills(usable_skills(resolve_skills(
        &SkillSources {
            cli: &args.skills,
            env: &env_skills(),
            config: &chat_config.skills,
        },
        dirs::home_dir().as_deref(),
    )));
    let plugin = write_skill_plugin(&skills)?;
    let plugin_args = plugin
        .as_ref()
        .map(|plugin| build_plugin_args(plugin.path()))
        .unwrap_or_default();
    let theme_config = ThemeConfig::from_config_and_env();
    config.theme.warnings().into_iter().for_each(warn);
    let agent = config.theme.agent();
    let agent_highlighter =
        Highlighter::with_config(theme_config.clone().merge_cli(agent.code.as_deref(), false))
            .unwrap_or_else(|e| {
                warn(format!("Failed to load agent theme: {e}"));
                Highlighter::new()
            });
    let agent_theme = MarkdownTheme::new(agent.palette(ThemeRole::Agent).0);
    let turn = Turn {
        store: &store,
        store_dir: &store_dir,
        mcp_config_path: &mcp_config_path,
        permission_mode: &permission_mode,
        plugin_args: &plugin_args,
        skills: &skills,
        timeout_secs: args.timeout,
        theme_config: &theme_config,
        frozen: Mutex::new(None),
        frozen_window,
        cancel: signal::interrupt_flag(),
    };
    let inbox = Arc::new(Inbox::new(store_dir.join(INBOX_DIR)));
    let known = inbox.peek().unwrap_or_default();
    let monitor = Arc::new(Monitor {
        watches: Watches::new(Arc::clone(&inbox), &store_dir),
        inbox,
        store: Store::new(store_dir.clone()),
        store_dir: canonical_store(store_dir.clone()),
        status: Mutex::new(Status::default()),
        seen: Mutex::new(HashMap::new()),
        active: Mutex::new(None),
        frozen_window,
        worker_check_secs,
        workers: Workers::new(
            store_dir.clone(),
            exe.clone(),
            permission_mode.clone(),
            args.worktree_dir.clone(),
        ),
        check: Mutex::new(CheckState::default()),
        known: Mutex::new(known),
        alert: AlertConfig {
            sound: chat_config.inbox_alert_sound && !args.no_inbox_sound,
            notification: chat_config.inbox_alert_notification && !args.no_inbox_notification,
            unknown_focus: chat_config.inbox_alert_unknown_focus,
        },
    });
    let _close = monitor.watches.close_guard();
    monitor.watches.restore();
    monitor.refresh(true);
    if let Some(hint) = orphan_hint(
        &ready_ids(&monitor.store_dir),
        &running_workers(&monitor.store_dir),
    ) {
        println!("{hint}");
    }
    let at_prompt = Arc::new(AtomicBool::new(false));
    watch_input(
        Arc::clone(&at_prompt),
        plugin.as_ref().map(|plugin| plugin.path().to_path_buf()),
        monitor.watches.clone(),
    );
    let mut editor = build_editor(&store_dir)?
        .with_wrap_width(wrap_cap.map(|cap| u16::try_from(cap).unwrap_or(u16::MAX)));
    let repaint = editor.repaint_signal();
    watch_status(Arc::clone(&monitor), move || repaint.request_repaint());
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let prompt = ChatPrompt {
        monitor: Arc::clone(&monitor),
        color,
        notice: RefCell::new(None),
    };
    let inbox_display = |color| Display {
        ctx: RenderContext::new(&agent_highlighter, color),
        theme: &agent_theme,
        color,
    };
    let mut shell_state = chat_cell::ShellState::default();
    loop {
        warn_if_err(editor.sync_history(), "Failed to save chat history");
        at_prompt.store(true, Ordering::SeqCst);
        let route = next_route(&mut editor, &prompt);
        at_prompt.store(false, Ordering::SeqCst);
        let Some(route) = route? else {
            break;
        };
        let outcome = match route {
            Route::Ignore => continue,
            Route::Send(message) => {
                let outcome = match chat_cell::prepare(&message, cell_limits, &mut shell_state) {
                    Ok(prepared) => {
                        if let Some(shown) = &prepared.shown {
                            let ctx = RenderContext::new(&agent_highlighter, color);
                            print!("{}", render_markdown(&ctx, shown, &agent_theme));
                        }
                        let sent = prepared.sent;
                        turn.run(Trigger::User, std::slice::from_ref(&sent), &sent)?
                    }
                    Err(outcome) => outcome,
                };
                if let Some(draft) = draft_after(outcome, message) {
                    editor.run_edit_commands(&[EditCommand::InsertString(draft)]);
                }
                outcome
            }
            Route::ReadInbox(selection) => {
                let reports = monitor.drain(selection)?;
                monitor.refresh(true);
                let Some(unread) = chat_inbox::read(selection, &reports, &inbox_display(color))
                else {
                    continue;
                };
                turn.run(Trigger::Worker, &unread.logged, &unread.message)?
            }
            local => {
                let ctx = LocalRoute {
                    inbox: &monitor.inbox,
                    watches: &monitor.watches,
                    display: &inbox_display(color),
                };
                run_local(local, &ctx, || {
                    monitor.refresh(true);
                })?;
                continue;
            }
        };
        monitor.refresh(true);
        if outcome == TurnOutcome::Exit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
