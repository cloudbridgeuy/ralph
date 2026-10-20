pub(crate) mod cell;
mod changes;
mod check;
#[cfg(test)]
mod corrupt_tests;
mod diff;
mod editor;
mod inbox;
mod index_files;
pub(crate) mod mcp;
pub(crate) mod memory;
mod monitor;
mod prompt;
mod route;
mod skills;
mod snapshots;
pub(crate) mod socket;
mod statusbar;
mod turn;
mod watch;

use crate::chat::check::CheckState;
use crate::chat::editor::build_editor;
use crate::chat::inbox::Display;
use crate::chat::memory::Store;
use crate::chat::monitor::{ready_ids, watch_headless, watch_input, watch_status, Monitor};
use crate::chat::prompt::{next_route, ChatPrompt};
use crate::chat::route::{run_local, LocalRoute};
use crate::chat::skills::{
    env_skills, link_skills, usable_skills, write_mcp_config, write_skill_plugin,
};
use crate::chat::snapshots::TurnSnapshots;
use crate::chat::socket::{ChatSocket, SocketGuard};
use crate::chat::statusbar::StatusBar;
use crate::chat::turn::{draft_after, Turn, TurnOutcome};
use crate::cli::ChatArgs;
use crate::config::AppConfig;
use crate::highlight::{Highlighter, ThemeConfig};
use crate::markdown::{MarkdownTheme, ThemeRole};
use crate::opencode_worker::sweep_stale_server_records;
use crate::render::{cap_wrap_width, render_markdown, RenderContext};
use crate::signal;
use crate::warn::{warn, warn_if_err};
use crate::watches::Watches;
use crate::worker_mcp::Inbox;
use crate::workers::headless::ExitCleanup;
use crate::workers::{canonical_store, running_workers, Workers};
use ralph_core::chat::{
    resolve_frozen_window, resolve_recent_history_tokens, resolve_watch_timeout,
    resolve_worker_check_secs, resolve_wrap_width,
};
use ralph_core::chat_cell::resolve_cell_limits;
use ralph_core::chat_draft::editor_argv;
use ralph_core::chat_edit_mode::resolve_edit_mode;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_skills::{build_plugin_args, resolve_skills, SkillSources};
use ralph_core::chat_status::{orphan_hint, Route, Status};
use ralph_core::inbox_alert::AlertConfig;
use ralph_core::snapshot::snapshots_enabled;
use ralph_core::worker::{ATTACHMENTS_DIR, INBOX_DIR};
use reedline::EditCommand;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const STORE_ENV: &str = "RALPH_CHAT_STORE";
const WRAP_WIDTH_ENV: &str = "RALPH_WRAP_WIDTH";
const CELL_OUTPUT_LIMIT_ENV: &str = "RALPH_CELL_OUTPUT_LIMIT";
const CELL_OUTPUT_BYTES_ENV: &str = "RALPH_CELL_OUTPUT_BYTES";

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

pub fn run(args: ChatArgs) -> Result<(), Box<dyn std::error::Error>> {
    warn_if_err(signal::init(), "Failed to initialize signal handler");
    signal::ignore_sigttou();
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
    let recent_history_tokens = resolve_recent_history_tokens(
        args.recent_history_tokens,
        chat_config.recent_history_tokens,
    );
    let worker_check_secs = resolve_worker_check_secs(chat_config.worker_check_secs);
    let (worker_quiet_secs, quiet_warning) = chat_config.worker_quiet_setting();
    let watch_timeout = resolve_watch_timeout(chat_config.watch_timeout_secs)?;
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
    sweep_stale_server_records();
    let chat_socket = ChatSocket::bind(&store_dir)?;
    let socket_files = chat_socket.files().clone();
    let _socket_guard = SocketGuard(socket_files.clone());
    let mcp_args = write_mcp_config(
        &store,
        &store_dir,
        chat_socket.path(),
        &exe,
        &chat_config.mcp_servers,
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
    config.changes.warnings().into_iter().for_each(warn);
    let (edit_mode, edit_mode_warning) = resolve_edit_mode(chat_config.edit_mode.as_deref());
    edit_mode_warning.into_iter().for_each(warn);
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
        mcp_args: &mcp_args,
        permission_mode: &permission_mode,
        plugin_args: &plugin_args,
        skills: &skills,
        theme_config: &theme_config,
        frozen: Mutex::new(None),
        frozen_window,
        recent_history_tokens,
        cancel: signal::interrupt_flag(),
    };
    let snapshots = TurnSnapshots::open(
        snapshots_enabled(Some(config.changes.enabled)),
        &store_dir,
        &std::env::current_dir()?,
    );
    let inbox = Arc::new(Inbox::new(store_dir.join(INBOX_DIR)));
    let known = inbox.peek().unwrap_or_default();
    let monitor = Arc::new(Monitor {
        watches: Watches::new(Arc::clone(&inbox), &store_dir, watch_timeout),
        inbox,
        store: Store::new(store_dir.clone()),
        store_dir: canonical_store(store_dir.clone()),
        status: Mutex::new(Status::default()),
        seen: Mutex::new(HashMap::new()),
        active: Mutex::new(None),
        frozen_window,
        recent_history_tokens,
        worker_check_secs,
        worker_quiet_secs,
        workers: Workers::new(
            store_dir.clone(),
            exe.clone(),
            permission_mode.clone(),
            args.worktree_dir.clone(),
        ),
        check: Mutex::new(CheckState::seeded(&store)),
        known: Mutex::new(known),
        warned: Mutex::new(HashSet::new()),
        alert: AlertConfig {
            sound: chat_config.inbox_alert_sound && !args.no_inbox_sound,
            notification: chat_config.inbox_alert_notification && !args.no_inbox_notification,
            unknown_focus: chat_config.inbox_alert_unknown_focus,
        },
    });
    chat_socket.serve(
        Store::new(store_dir.clone()),
        Workers::new(
            store_dir.clone(),
            exe.clone(),
            permission_mode.clone(),
            args.worktree_dir.clone(),
        ),
    );
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
    let headless = watch_headless(monitor.store_dir.clone());
    watch_input(
        Arc::clone(&at_prompt),
        plugin.as_ref().map(|plugin| plugin.path().to_path_buf()),
        ExitCleanup::new(monitor.watches.clone(), &headless, socket_files),
    );
    let mut editor = build_editor(&store_dir, edit_mode)?
        .with_wrap_width(wrap_cap.map(|cap| u16::try_from(cap).unwrap_or(u16::MAX)));
    let repaint = editor.repaint_signal();
    watch_status(Arc::clone(&monitor), move || repaint.request_repaint());
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let (configured_editor, config_warning) = chat_config.editor_setting();
    let (editor_cmd, editor_warning) = editor_argv(
        configured_editor,
        std::env::var("VISUAL").ok().as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
    );
    let (empty_submit, empty_warning) = chat_config.empty_submit_setting();
    config_warning
        .into_iter()
        .chain(editor_warning)
        .chain(quiet_warning)
        .chain(empty_warning)
        .for_each(warn);
    let prompt = ChatPrompt {
        monitor: Arc::clone(&monitor),
        color,
        bar: StatusBar::new(
            &config.theme.status,
            &monitor.store_dir,
            &chat_config,
            color,
        ),
        notice: RefCell::new(None),
        editor: editor_cmd,
        empty_submit,
    };
    let inbox_display = |color| Display {
        ctx: RenderContext::new(&agent_highlighter, color),
        theme: &agent_theme,
        color,
    };
    let mut shell_state = cell::ShellState::default();
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
                let outcome = match cell::prepare(&message, cell_limits, &mut shell_state) {
                    Ok(prepared) => {
                        if let Some(shown) = &prepared.shown {
                            let ctx = RenderContext::new(&agent_highlighter, color);
                            print!("{}", render_markdown(&ctx, shown, &agent_theme));
                        }
                        let sent = prepared.sent;
                        snapshots.around(|| {
                            turn.run(Trigger::User, std::slice::from_ref(&sent), &sent)
                        })?
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
                let Some(unread) = inbox::read(selection, &reports, &inbox_display(color)) else {
                    continue;
                };
                snapshots.around(|| turn.run(Trigger::Worker, &unread.logged, &unread.message))?
            }
            local => {
                let ctx = LocalRoute {
                    inbox: &monitor.inbox,
                    watches: &monitor.watches,
                    display: &inbox_display(color),
                    changes: &config.changes,
                    store: &store_dir,
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
mod tests;
