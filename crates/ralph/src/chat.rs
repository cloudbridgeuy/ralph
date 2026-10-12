use crate::ansi::{DIM, RESET};
use crate::chat_memory::Store;
use crate::cli::ChatArgs;
use crate::config::AppConfig;
use crate::highlight::ThemeConfig;
use crate::inbox_alert;
use crate::invoke::build_chat_command;
use crate::markdown::create_markdown_skin;
use crate::signal;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::extract_response_text;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, SubprocessError,
};
use crate::warn::{warn, warn_if_err};
use crate::worker_mcp::Inbox;
use crate::workers::{canonical_store, opencode_active, tmux_listing};
use ralph_core::chat::{
    build_agents_args, build_mcp_args, mcp_config, resolve_frozen_window, select_servers,
    turn_prompt, turn_system_prompt, VIEW_LINES,
};
use ralph_core::chat_memory::{
    frozen_block, tail_lines, usage_record, view_stats, CallKind, Frozen, Role, Trigger,
};
use ralph_core::chat_skills::{
    build_plugin_args, expand_slash_command, plan_links, plugin_manifest, resolve_skills,
    ChatSkill, SkillSources,
};
use ralph_core::chat_status::{
    footer, last_turn_cache, latest_states, memory_size, orphan_hint, route_input, status_line,
    store_workers, worker_states, Memory, Route, RunningWorker, Status,
};
use ralph_core::inbox_alert::{fresh_reports, inbox_alert, AlertConfig};
use ralph_core::worker::{chat_message, Report, INBOX_DIR, READY_DIR};
use reedline::{
    default_emacs_keybindings, EditCommand, Emacs, FileBackedHistory, KeyCode, KeyModifiers,
    Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline,
    ReedlineEvent, Signal, DEFAULT_MULTILINE_INDICATOR,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

const PROMPT: &str = ">>> ";
const HISTORY_FILE: &str = "history.txt";
const EDITOR_FILE: &str = "editor.md";
const HISTORY_CAPACITY: usize = 1000;
const SKILLS_ENV: &str = "RALPH_CHAT_SKILLS";
const POLL: Duration = Duration::from_millis(500);
const MEMORY_EVERY_POLLS: u32 = 10;
const DEFAULT_TERMINAL_WIDTH: usize = 80;

fn default_store() -> io::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".ralph").join("chat"))
        .ok_or_else(|| io::Error::other("home directory not found"))
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
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    Ok(Reedline::create()
        .with_history(Box::new(history))
        .with_edit_mode(Box::new(Emacs::new(keybindings)))
        .with_buffer_editor(Command::new(editor), store_dir.join(EDITOR_FILE))
        .use_bracketed_paste(true))
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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

struct Monitor {
    inbox: Inbox,
    store: Store,
    store_dir: PathBuf,
    status: Mutex<Status>,
    seen: Mutex<HashMap<String, String>>,
    active: Mutex<Option<HashSet<String>>>,
    frozen_window: usize,
    known: Mutex<Vec<Report>>,
    alert: AlertConfig,
}

impl Monitor {
    fn status(&self) -> Status {
        locked(&self.status).clone()
    }

    fn drain(&self) -> io::Result<Vec<Report>> {
        let reports = self.inbox.drain()?;
        locked(&self.seen).extend(latest_states(&reports));
        Ok(reports)
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
        for poll in (1..=MEMORY_EVERY_POLLS).cycle() {
            std::thread::sleep(POLL);
            if monitor.refresh(poll == MEMORY_EVERY_POLLS) {
                repaint();
            }
        }
    });
}

struct ChatPrompt {
    monitor: Arc<Monitor>,
    color: bool,
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

fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(w, _)| usize::from(w))
        .unwrap_or(DEFAULT_TERMINAL_WIDTH)
}

impl Prompt for ChatPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Owned(self.dim(&status_line(&self.monitor.status(), terminal_width())))
    }

    fn render_prompt_footer(&self) -> Cow<'_, str> {
        let hints = footer(self.monitor.status().unread, terminal_width());
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
    Ok(match editor.read_line(prompt)? {
        Signal::Success(line) => Some(route_input(&line, prompt.monitor.status().unread)),
        _ => None,
    })
}

fn print_rule() {
    print!("{}", create_markdown_skin().term_text("---"));
}

fn spawn_nap(store: &Path) -> io::Result<()> {
    Command::new(std::env::current_exe()?)
        .arg("chat-nap")
        .arg("--store")
        .arg(store)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
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
}

impl Turn<'_> {
    fn run(
        &self,
        trigger: Trigger,
        logged: &[String],
        message: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let (entries, tree) = self.store.load_settled()?;
        let pinned = locked(&self.frozen).take();
        let frozen = frozen_block(&entries, &tree, VIEW_LINES, self.frozen_window, pinned);
        *locked(&self.frozen) = Some(frozen.clone());
        let tail = tail_lines(&entries, &tree, frozen.end);
        for text in logged {
            self.store.append(trigger.role(), text)?;
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
            session_info: SpinnerSessionInfo {
                persona: Some("chat".into()),
            },
            pause: None,
            cancel: None,
        });
        let result = match result {
            Ok(result) => result,
            Err(SubprocessError::Interrupted { .. }) => return Ok(false),
            Err(SubprocessError::Timeout { timeout_secs, .. }) => {
                print_rule();
                warn(format!(
                    "chat turn timed out after {timeout_secs}s; restart with a larger --timeout for long work"
                ));
                return Ok(true);
            }
            Err(e) => return Err(e.into()),
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
                self.store.append(Role::Assistant, &reply)?;
                warn_if_err(spawn_nap(self.store_dir), "Failed to start chat memory nap");
            }
            _ => warn(format!("chat turn failed (exit {})", result.exit_code)),
        }
        Ok(true)
    }
}

fn write_mcp_config(
    store: &Store,
    store_dir: &Path,
    exe: &Path,
    server_names: &[String],
    permission_mode: &str,
) -> io::Result<PathBuf> {
    let claude_config = read_claude_config();
    let imported = select_servers(&claude_config, server_names);
    for name in &imported.missing {
        warn(format!(
            "chat: MCP server \"{name}\" not found in ~/.claude.json; continuing without it"
        ));
    }
    store.write_mcp_config(&mcp_config(
        exe,
        store_dir,
        permission_mode,
        &imported.servers,
    ))
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
    let store_dir = match args.store {
        Some(dir) => dir,
        None => default_store()?,
    };
    std::fs::create_dir_all(&store_dir)?;
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
    let mcp_config_path = write_mcp_config(
        &store,
        &store_dir,
        &exe,
        &chat_config.mcp_servers,
        &permission_mode,
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
    };
    let inbox = Inbox::new(store_dir.join(INBOX_DIR));
    let known = inbox.peek().unwrap_or_default();
    let monitor = Arc::new(Monitor {
        inbox,
        store: Store::new(store_dir.clone()),
        store_dir: canonical_store(store_dir.clone()),
        status: Mutex::new(Status::default()),
        seen: Mutex::new(HashMap::new()),
        active: Mutex::new(None),
        frozen_window,
        known: Mutex::new(known),
        alert: AlertConfig {
            sound: chat_config.inbox_alert_sound && !args.no_inbox_sound,
            notification: chat_config.inbox_alert_notification && !args.no_inbox_notification,
            unknown_focus: chat_config.inbox_alert_unknown_focus,
        },
    });
    monitor.refresh(true);
    if let Some(hint) = orphan_hint(
        &ready_ids(&monitor.store_dir),
        &running_workers(&monitor.store_dir),
    ) {
        println!("{hint}");
    }
    let mut editor = build_editor(&store_dir)?;
    let repaint = editor.repaint_signal();
    watch_status(Arc::clone(&monitor), move || repaint.request_repaint());
    let prompt = ChatPrompt {
        monitor: Arc::clone(&monitor),
        color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    };
    while let Some(route) = next_route(&mut editor, &prompt)? {
        let keep_going = match route {
            Route::Ignore => continue,
            Route::Send(message) => {
                turn.run(Trigger::User, std::slice::from_ref(&message), &message)?
            }
            Route::ReadInbox => {
                let reports = monitor.drain()?;
                monitor.refresh(true);
                if reports.is_empty() {
                    println!("No unread reports.");
                    continue;
                }
                let logged: Vec<String> = reports
                    .iter()
                    .map(|report| chat_message(std::slice::from_ref(report)))
                    .collect();
                let message = chat_message(&reports);
                println!("{message}");
                turn.run(Trigger::Worker, &logged, &message)?
            }
        };
        monitor.refresh(true);
        if !keep_going {
            break;
        }
    }
    Ok(())
}
