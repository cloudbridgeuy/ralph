pub(crate) mod cell;
mod changes;
mod check;
#[cfg(test)]
mod corrupt_tests;
mod diff;
mod editor;
mod hold;
mod inbox;
mod index_files;
mod keepalive;
mod keys;
pub(crate) mod mcp;
pub(crate) mod memory;
mod monitor;
mod prompt;
mod queue;
mod route;
mod schedule;
mod schedule_worker;
mod skills;
pub(crate) use skills::SKILLS_ENV;
mod snapshots;
pub(crate) mod socket;
mod statusbar;
mod suggestion;
pub(crate) mod terminal;
mod turn;
mod watch;
mod workers;
mod workers_remote;

use crate::chat::check::CheckState;
use crate::chat::editor::build_editor;
use crate::chat::hold::Held;
use crate::chat::inbox::Display;
use crate::chat::keys::Keys;
use crate::chat::memory::Store;
use crate::chat::monitor::{
    ready_ids, watch_headless, watch_input, watch_status, Monitor, ThreadMemo,
};
use crate::chat::prompt::{next_route, ChatPrompt, Submitted};
use crate::chat::queue::{
    decide, draft_with_queue, draft_with_typed, exit_notice, unsent, Decision, Queue,
};
use crate::chat::route::{run_local, LocalRoute};
use crate::chat::schedule::{approve_proposed, restore_schedule};
use crate::chat::skills::{
    env_skills, link_skills, read_claude_config, usable_skills, write_mcp_config,
    write_skill_plugin,
};
use crate::chat::snapshots::TurnSnapshots;
use crate::chat::socket::{ChatSocket, SocketGuard};
use crate::chat::statusbar::StatusBar;
use crate::chat::suggestion::{SuggestionHighlighter, SuggestionSlot};
use crate::chat::turn::{draft_after, Turn, TurnOutcome};
use crate::chat::workers_remote::RemoteStatus;
use crate::cli::ChatArgs;
use crate::config::{AppConfig, PromptFiles};
use crate::cursor_owner::{acp_alive_ids, resumable_ids};
use crate::highlight::{Highlighter, ThemeConfig};
use crate::input_theme::suggestion_color;
use crate::invoke::Isolation;
use crate::markdown::{MarkdownTheme, ThemeRole};
use crate::opencode_worker::sweep_stale_server_records;
use crate::render::{cap_wrap_width, RenderContext};
use crate::schedule_store::ScheduleStore;
use crate::signal;
use crate::subprocess::OutputGate;
use crate::warn::{warn, warn_if_err};
use crate::watches::Watches;
use crate::worker_mcp::Inbox;
use crate::workers::headless::ExitCleanup;
use crate::workers::{canonical_store, running_workers, Workers};
use keepalive::{KeepaliveWatch, Stamp};
use ralph_core::chat::{
    prompt_text, resolve_recent_history_tokens, resolve_watch_timeout, resolve_worker_check_secs,
    resolve_wrap_width, select_servers, CustomPrompt, ReplyImages,
};
use ralph_core::chat_cell::resolve_cell_limits;
use ralph_core::chat_draft::editor_argv;
use ralph_core::chat_edit_mode::resolve_edit_mode;
use ralph_core::chat_keepalive::resolve_keepalive_hours;
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_skills::{build_plugin_args, resolve_skills, SkillSources};
use ralph_core::chat_status::{orphan_hint, route_input, Route, Status};
use ralph_core::chat_suggestion::{prefill, Prefill, SuggestionRecord};
use ralph_core::chat_thread::{run_id, UsageTail};
use ralph_core::inbox::Selection;
use ralph_core::inbox_alert::AlertConfig;
use ralph_core::schedule_request::SCHEDULE_DIR;
use ralph_core::snapshot::snapshots_enabled;
use ralph_core::worker::{ATTACHMENTS_DIR, INBOX_DIR};
use reedline::{EditCommand, HistoryItem};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const STORE_ENV: &str = "RALPH_CHAT_STORE";
pub(crate) const WRAP_WIDTH_ENV: &str = "RALPH_WRAP_WIDTH";
pub(crate) const CELL_OUTPUT_LIMIT_ENV: &str = "RALPH_CELL_OUTPUT_LIMIT";
pub(crate) const CELL_OUTPUT_BYTES_ENV: &str = "RALPH_CELL_OUTPUT_BYTES";

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

fn read_prompt_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("Skipping prompt file {}: {e}", path.display()))
}

fn load_custom_prompt(files: &PromptFiles) -> (CustomPrompt, Vec<String>) {
    let (behaviour, behaviour_warning) = match files.behaviour.as_deref() {
        Some(path) => match read_prompt_file(path) {
            Ok(text) => match prompt_text(text) {
                Some(text) => (Some(text), None),
                None => (
                    None,
                    Some(format!(
                        "Prompt file {} is empty; using the default reply rules",
                        path.display()
                    )),
                ),
            },
            Err(warning) => (None, Some(warning)),
        },
        None => (None, None),
    };
    let (appends, append_warnings): (Vec<_>, Vec<_>) = files
        .appends
        .iter()
        .map(|path| read_prompt_file(path))
        .partition(Result::is_ok);
    (
        CustomPrompt {
            behaviour,
            appends: appends.into_iter().flatten().collect(),
        },
        behaviour_warning
            .into_iter()
            .chain(append_warnings.into_iter().filter_map(Result::err))
            .collect(),
    )
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
    crate::config::set_store_layer(&store_dir);
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
    let recent_history_tokens = resolve_recent_history_tokens(
        args.recent_history_tokens,
        chat_config.recent_history_tokens,
    );
    let worker_check_secs = resolve_worker_check_secs(chat_config.worker_check_secs);
    let keepalive_hours = resolve_keepalive_hours(chat_config.keepalive_hours);
    let (worker_quiet_secs, quiet_warning) = chat_config.worker_quiet_setting();
    let (reset_limits, reset_warnings) = chat_config.reset_limits_setting();
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
    let suggestions = SuggestionSlot::default();
    let shown = SuggestionSlot::default();
    let socket_files = chat_socket.files().clone();
    let _socket_guard = SocketGuard(socket_files.clone());
    let imported = select_servers(
        &read_claude_config(config.claude.config_dir.as_deref()),
        &chat_config.mcp_servers,
    );
    let mcp_args = write_mcp_config(&store, &store_dir, chat_socket.path(), &exe, &imported)?;
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
    config.theme.deprecations().into_iter().for_each(warn);
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
    let (custom_prompt, prompt_warnings) =
        load_custom_prompt(&chat_config.prompt_files(dirs::home_dir().as_deref()));
    prompt_warnings.into_iter().for_each(warn);
    let isolation = Isolation::for_chat(
        &chat_config,
        &std::env::current_dir()?,
        config.claude.config_dir.as_deref(),
    );
    let live = Arc::new(AtomicBool::new(false));
    let run = run_id(std::process::id(), chrono::Utc::now());
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let image_settings = crate::render::image::detect(color);
    let reply_images = if image_settings.is_some() {
        ReplyImages::Inline
    } else {
        ReplyImages::Text
    };
    let last_turn_end: Stamp = Arc::default();
    let last_ping: Stamp = Arc::default();
    let turn = Turn {
        store: &store,
        store_dir: &store_dir,
        mcp_args: &mcp_args,
        permission_mode: &permission_mode,
        plugin_args: &plugin_args,
        skills: &skills,
        custom_prompt: &custom_prompt,
        reply_images,
        isolation: &isolation,
        theme_config: &theme_config,
        session: Arc::new(Mutex::new(None)),
        process_turns: AtomicU64::new(0),
        reset: Mutex::new(None),
        live: Arc::clone(&live),
        reset_limits,
        recent_history_tokens,
        cancel: signal::interrupt_flag(),
        gate: Arc::new(OutputGate::new()),
        run: &run,
        suggestions: suggestions.clone(),
        last_turn_end: Arc::clone(&last_turn_end),
        identity: Arc::default(),
    };
    let snapshots = TurnSnapshots::open(
        snapshots_enabled(Some(config.changes.enabled)),
        &store_dir,
        &std::env::current_dir()?,
    );
    let inbox = Arc::new(Inbox::new(store_dir.join(INBOX_DIR)));
    let known = inbox.peek().unwrap_or_default();
    let usage_stamp = memory::usage_stamp(&memory::usage_path()).unwrap_or_default();
    let monitor = Arc::new(Monitor {
        watches: Watches::new(Arc::clone(&inbox), &store_dir, watch_timeout),
        schedule: ScheduleStore::new(store_dir.join(SCHEDULE_DIR)),
        inbox,
        store: Store::new(store_dir.clone()),
        store_dir: canonical_store(store_dir.clone()),
        status: Mutex::new(Status::default()),
        seen: Mutex::new(HashMap::new()),
        active: Mutex::new(None),
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
        scheduled_reports: Mutex::new(Vec::new()),
        warned: Mutex::new(HashSet::new()),
        alert: AlertConfig {
            sound: chat_config.inbox_alert_sound && !args.no_inbox_sound,
            notification: chat_config.inbox_alert_notification && !args.no_inbox_notification,
            unknown_focus: chat_config.inbox_alert_unknown_focus,
        },
        live,
        last_ping: Arc::clone(&last_ping),
        run: run.clone(),
        reset_limits,
        thread: Mutex::new(ThreadMemo::default()),
        usage_tail: Mutex::new(UsageTail::starting_at(&usage_stamp)),
    });
    chat_socket.serve(
        Store::new(store_dir.clone()),
        Workers::new(
            store_dir.clone(),
            exe.clone(),
            permission_mode.clone(),
            args.worktree_dir.clone(),
        ),
        suggestions.clone(),
    );
    let _close = monitor.watches.close_guard();
    monitor.watches.restore();
    restore_schedule(
        &monitor.schedule,
        &schedule_worker::alive_worker_ids(&monitor.store_dir),
    );
    monitor.refresh(true);
    if let Some(hint) = orphan_hint(
        &ready_ids(&monitor.store_dir),
        &running_workers(&monitor.store_dir),
        &acp_alive_ids(&monitor.store_dir),
        &resumable_ids(&monitor.store_dir),
    ) {
        println!("{hint}");
    }
    let at_prompt = Arc::new(AtomicBool::new(false));
    let idle = Arc::new(AtomicBool::new(false));
    if let Some(limit_hours) = keepalive_hours {
        KeepaliveWatch {
            limit_hours,
            idle: Arc::clone(&idle),
            session: Arc::clone(&turn.session),
            live: Arc::clone(&turn.live),
            last_turn_end,
            last_ping,
            last_attempt: Arc::new(Mutex::new(None)),
            keeps: Mutex::default(),
            identity: Arc::clone(&turn.identity),
            run: run.clone(),
            store: store_dir.to_string_lossy().into_owned(),
        }
        .spawn();
    }
    let headless = watch_headless(monitor.store_dir.clone());
    watch_input(
        Arc::clone(&at_prompt),
        Arc::clone(&turn.session),
        plugin.as_ref().map(|plugin| plugin.path().to_path_buf()),
        ExitCleanup::new(monitor.watches.clone(), &headless, socket_files),
    );
    let highlighter = suggestion_color(&config.theme.input.resolve().0, color)
        .map(|suggestion| SuggestionHighlighter::new(shown.clone(), suggestion));
    let mut editor = build_editor(&store_dir, edit_mode, highlighter)?
        .with_wrap_width(wrap_cap.map(|cap| u16::try_from(cap).unwrap_or(u16::MAX)));
    let repaint = editor.repaint_signal();
    watch_status(Arc::clone(&monitor), move || repaint.request_repaint());
    crate::render::image::enable_images(image_settings);
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
        .chain(reset_warnings)
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
    let remote = RemoteStatus::new();
    let mut shell_state = cell::ShellState::default();
    let display = inbox_display(color);
    let local_route = LocalRoute {
        inbox: &monitor.inbox,
        watches: &monitor.watches,
        schedule: &monitor.schedule,
        display: &display,
        changes: &config.changes,
        store: &store_dir,
        monitor: &monitor,
        remote: &remote,
    };
    let keys = Keys {
        gate: &turn.gate,
        cancel: &turn.cancel,
        monitor: &monitor,
        bar: &prompt.bar,
        edit_mode,
        local: &local_route,
        empty_submit: &prompt.empty_submit,
    };
    let schedule_store = monitor.schedule.clone();
    approve_proposed(&schedule_store, watch_timeout);
    let mut pending: Option<Route> = None;
    let mut offered: Option<(String, u64)> = None;
    loop {
        warn_if_err(editor.sync_history(), "Failed to save chat history");
        let route = match pending.take() {
            Some(route) => Some(route),
            None => {
                at_prompt.store(true, Ordering::SeqCst);
                idle.store(true, Ordering::SeqCst);
                let submitted = next_route(&mut editor, &prompt);
                at_prompt.store(false, Ordering::SeqCst);
                idle.store(false, Ordering::SeqCst);
                submitted?.map(|Submitted { line, route }| {
                    if let Some((suggestion, turn_number)) = offered.take() {
                        let record = SuggestionRecord::new(
                            chrono::Utc::now().to_rfc3339(),
                            &run,
                            turn_number,
                            &suggestion,
                            &line,
                        );
                        warn_if_err(
                            store.append_suggestion(&record),
                            "Failed to record suggestion outcome",
                        );
                    }
                    route
                })
            }
        };
        let Some(route) = route else {
            break;
        };
        let mut queue = Queue::default();
        let (outcome, draft) = match route {
            Route::Ignore => continue,
            Route::Send(message) => {
                if let Some(images) = crate::render::image::enabled() {
                    print!(
                        "{}",
                        crate::render::image::render_attachments(
                            &message,
                            images,
                            crate::render::image::ImageBounds::current()
                        )
                    );
                }
                let outcome = match cell::prepare(&message, cell_limits, &mut shell_state) {
                    Ok(prepared) => {
                        if let Some(shown) = &prepared.shown {
                            let ctx = RenderContext::new(&agent_highlighter, color);
                            print!(
                                "{}",
                                crate::render::image::render_cell_display(
                                    &ctx,
                                    shown,
                                    &agent_theme
                                )
                            );
                        }
                        let sent = prepared.sent;
                        let (result, queued) = snapshots.around(|| {
                            keys.during(|| {
                                turn.run(Trigger::User, std::slice::from_ref(&sent), &sent)
                            })
                        });
                        queue = queued;
                        result?
                    }
                    Err(outcome) => outcome,
                };
                (outcome, draft_after(outcome, message))
            }
            Route::ReadInbox(selection) => {
                let reports = monitor.drain(selection)?;
                monitor.refresh(true);
                let Some(unread) = inbox::read(selection, &reports, &inbox_display(color)) else {
                    continue;
                };
                let (result, queued) = snapshots.around(|| {
                    keys.during(|| turn.run(Trigger::Worker, &unread.logged, &unread.message))
                });
                queue = queued;
                (result?, None)
            }
            Route::Hold(filter) => {
                idle.store(true, Ordering::SeqCst);
                let held = hold::run(&filter, &monitor, &prompt.bar);
                idle.store(false, Ordering::SeqCst);
                match held {
                    Held::Item(position) => {
                        pending = Some(Route::ReadInbox(Selection::At(position)));
                    }
                    Held::Left => {}
                    Held::Quit => break,
                }
                continue;
            }
            local => {
                run_local(local, &local_route, || {
                    monitor.refresh(true);
                })?;
                continue;
            }
        };
        monitor.refresh(true);
        if outcome == TurnOutcome::Exit {
            let lost = unsent(&queue);
            for text in &lost {
                warn_if_err(
                    editor
                        .history_mut()
                        .save(HistoryItem::from_command_line(text.clone())),
                    "Failed to save chat history",
                );
            }
            if let Some(notice) = exit_notice(lost.len()) {
                eprintln!("{notice}");
            }
            editor.sync_history()?;
            break;
        }
        approve_proposed(&schedule_store, watch_timeout);
        let suggestion = suggestions.take();
        let typed = std::mem::take(&mut queue.typed);
        let (draft, suggestion) = match decide(outcome, queue) {
            Decision::Send(text) => {
                print!("{}", prompt.echo(&text));
                warn_if_err(
                    editor
                        .history_mut()
                        .save(HistoryItem::from_command_line(text.clone())),
                    "Failed to save chat history",
                );
                pending = Some(route_input(
                    &text,
                    monitor.status().unread,
                    &prompt.empty_submit,
                ));
                (draft, None)
            }
            Decision::Keep(entries) => (draft_with_queue(draft, &entries), suggestion),
        };
        let draft = draft_with_typed(draft, &typed);
        let replied = outcome == TurnOutcome::Replied;
        shown.clear();
        offered = None;
        if let Some(fill) = prefill(replied, draft, suggestion) {
            let (Prefill::Draft(text) | Prefill::Suggestion(text)) = &fill;
            editor.run_edit_commands(&[EditCommand::InsertString(text.clone())]);
            if let Prefill::Suggestion(text) = fill {
                offered = Some((text.clone(), turn.process_turns.load(Ordering::SeqCst)));
                shown.set(text);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
