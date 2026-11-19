mod bounded;
pub(crate) mod cell;
mod client_monitor;
mod config;
#[cfg(test)]
mod corrupt_tests;
mod diff;
mod direct;
mod editor;
mod hold;
mod hold_progress;
mod inbox;
mod keys;
pub(crate) mod mcp;
mod nudge;
mod prompt;
mod queue;
mod recap;
mod refusal;
mod rerender;
mod route;
mod schedule;
pub(crate) use ralph_server::chat::tag_store;
mod send_it;
mod sent;
pub(crate) mod session;
pub(crate) mod socket;
mod statusbar;
mod suggestion;
mod switch;
mod term_status;
pub(crate) mod terminal;
pub(crate) mod thread;
mod thread_cmd;
mod view;
mod workers;

use crate::chat::client_monitor::{
    client_sink, watch_headless, watch_input, watch_status, Monitor,
};
use crate::chat::editor::build_editor;
use crate::chat::hold::Held;
use crate::chat::inbox::Display;
use crate::chat::keys::Keys;
use crate::chat::prompt::{next_route, ChatPrompt, Submitted};
use crate::chat::queue::{
    decide, draft_with_queue, draft_with_typed, next_batch, unsent, unsent_texts, Decision, Queue,
};
use crate::chat::refusal::{refuse_or_exit, restore_prompt};
use crate::chat::route::{run_local, LocalRoute};
use crate::chat::schedule::approve_proposed;
use crate::chat::send_it::{worker_states, Active, InboxTurn, SendIt};
use crate::chat::socket::{ChatSocket, Shared, SocketGuard};
use crate::chat::statusbar::{BarContext, StatusBar};
use crate::chat::suggestion::SuggestionHighlighter;
use crate::chat::switch::{chat_exit, check as check_switch, Leave};
use crate::chat::term_status::{ClearOnDrop, TermStatus};
use crate::chat::workers::tool_report;
use crate::cli::{ChatArgs, ChatCommand};
use crate::config::{load_for_chat, ConfigSnapshot, FileState, MermaidStyleExt, ThemeWarnings};
use crate::highlight::{Highlighter, ThemeConfig};
use crate::input_theme::suggestion_color;
use crate::markdown::{MarkdownTheme, RolePalette, ThemeRole};
use crate::render::{cap_wrap_width, RenderContext};
use crate::signal;
use crate::theme_slots::ResolveColors;
use crate::warn::print_above_prompt;
use ralph_core::chat::{
    resolve_recent_history_tokens, resolve_watch_timeout, resolve_worker_check_secs,
    resolve_wrap_width, select_servers, ReplyImages,
};
use ralph_core::chat_cell::resolve_cell_limits;
use ralph_core::chat_close::LoopExit;
use ralph_core::chat_draft::editor_argv;
use ralph_core::chat_edit_mode::resolve_edit_mode;
use ralph_core::chat_keepalive::resolve_keepalive_hours;
use ralph_core::chat_local::CommandRecord;
use ralph_core::chat_memory::{store_in_use_message, Trigger};
use ralph_core::chat_process::{run_id, UsageTail};
use ralph_core::chat_send_it::{
    propose_prompt, usage as send_it_usage, Ended, Seen, SendItCommand,
};
use ralph_core::chat_skills::{build_plugin_args, resolve_skills, SkillSources};
use ralph_core::chat_socket::{runtime_root, system_prompt_path};
use ralph_core::chat_status::{orphan_hint, Route, Status};
use ralph_core::chat_suggestion::{
    auto_hold_notice, turn_end, Next, Prefill, SuggestionRecord, TurnEnd,
};
use ralph_core::chat_switch::SwitchArgv;
use ralph_core::claude_leftovers::claude_dir;
use ralph_core::inbox_alert::AlertConfig;
use ralph_core::program_status::{status_enabled, Phase};
use ralph_core::schedule_request::SCHEDULE_DIR;
use ralph_core::snapshot::snapshots_enabled;
use ralph_core::worker::{ATTACHMENTS_DIR, INBOX_DIR};
use ralph_server::chat::check::CheckState;
use ralph_server::chat::keepalive::{KeepaliveWatch, Stamp};
use ralph_server::chat::memory::Store;
use ralph_server::chat::monitor::{ready_ids, Monitor as Core, ProcessMemo};
use ralph_server::chat::schedule::restore_schedule;
use ralph_server::chat::skills::read_claude_config;
use ralph_server::chat::skills::{
    env_skills, link_skills, usable_skills, write_mcp_config, write_skill_plugin,
};
use ralph_server::chat::snapshots::TurnSnapshots;
use ralph_server::chat::suggestion::SuggestionCell;
use ralph_server::chat::turn::{draft_after, Turn, TurnOutcome};
use ralph_server::chat::workers_remote::RemoteStatus;
use ralph_server::chat::{memory, name};
use ralph_server::cursor_owner::{acp_alive_ids, resumable_ids};
use ralph_server::invoke::Isolation;
use ralph_server::opencode_worker::sweep_stale_server_records;
use ralph_server::paths::{absolute_from, config_path};
use ralph_server::schedule_store::ScheduleStore;
use ralph_server::warn::{warn, warn_if_err};
use ralph_server::watches::Watches;
use ralph_server::wire::store::current_uid;
use ralph_server::worker_mcp::Inbox;
use ralph_server::workers::headless::ExitCleanup;
use ralph_server::workers::{canonical_store, running_workers, usage_store, Workers};
use reedline::{EditCommand, ExternalPrinter};
#[cfg(test)]
use session::resolve_store;
use session::{
    chat_dir, load_custom_prompt, print_exit_notice, print_history, route_queued, save_unsent,
    store_dir,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
pub(crate) use switch::ChatExit;

const STORE_ENV: &str = "RALPH_CHAT_STORE";
const REX_SESSION_ENV: &str = "REX_SESSION";
const PROMPT_ROWS: usize = 256;
pub(crate) const WRAP_WIDTH_ENV: &str = "RALPH_WRAP_WIDTH";
pub(crate) const CELL_OUTPUT_LIMIT_ENV: &str = "RALPH_CELL_OUTPUT_LIMIT";
pub(crate) const CELL_OUTPUT_BYTES_ENV: &str = "RALPH_CELL_OUTPUT_BYTES";

pub fn run(args: ChatArgs) -> Result<ChatExit, Box<dyn std::error::Error>> {
    if let Some(ChatCommand::History(history)) = args.command {
        print_history(&chat_dir(args.store, args.thread.as_ref())?, &history)?;
        return Ok(chat_exit(LoopExit::History));
    }
    warn_if_err(signal::init(), "Failed to initialize signal handler");
    signal::ignore_sigttou();
    let flags = config::flag_settings(&args);
    let store_dir = chat_dir(args.store, args.thread.as_ref())?;
    std::fs::create_dir_all(store_dir.join(ATTACHMENTS_DIR))?;
    let store_root = canonical_store(store_dir.clone());
    ralph_server::config::set_store_layer(&store_dir);
    let store = Store::new(store_dir.clone());
    let _chat = store
        .claim_chat()?
        .map_err(|holder| store_in_use_message(&store_root, holder))?;
    let model_args =
        thread::open::model_args(&store_dir, args.thread.as_ref(), args.model.as_deref())?;
    let fork = thread::open_fork(&store_dir)?;
    let exe = std::env::current_exe()?;
    let cwd = std::env::current_dir()?;
    let global_config = absolute_from(&config_path(), &cwd);
    let store_config = thread::store_config_path(&store_dir);
    let loaded = load_for_chat(&global_config, &store_config);
    let config = loaded.config;
    let permission_mode = config.chat_permission_mode(
        args.permission_mode.as_deref(),
        std::env::var(ralph_server::invoke::CHAT_PERMISSION_MODE_ENV)
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
    let chat_socket = ChatSocket::bind(&store_dir, &runtime_root(current_uid()))?;
    let suggestions = SuggestionCell::default();
    let shown = SuggestionCell::default();
    let socket_files = chat_socket.files().clone();
    let _socket_guard = SocketGuard(socket_files.clone());
    let imported = select_servers(
        &read_claude_config(config.claude.config_dir.as_deref()),
        &chat_config.mcp_servers,
    );
    let mcp_args = write_mcp_config(&store_dir, chat_socket.path(), &exe, &imported)?;
    let system_prompt_file = system_prompt_path(chat_socket.path());
    let skill_dirs = resolve_skills(
        &SkillSources {
            cli: &args.skills,
            env: &env_skills(),
            config: &chat_config.skills,
        },
        dirs::home_dir().as_deref(),
    );
    let skills = link_skills(usable_skills(skill_dirs.clone()));
    let plugin = write_skill_plugin(&skills)?;
    let plugin_args = plugin
        .as_ref()
        .map(|plugin| build_plugin_args(plugin.path()))
        .unwrap_or_default();
    let theme_config = ThemeConfig::from_config_and_env();
    crate::warn::set_warning_color(theme_config.ui().warning);
    crate::warn::set_notice_color(theme_config.ui().ask_ok);
    let diff_colors = *theme_config.diff();
    let tool_colors = *theme_config.tools();
    let (edit_mode, edit_mode_warning) = resolve_edit_mode(chat_config.edit_mode.as_deref());
    let setting_warnings: Vec<String> = config
        .theme
        .deprecations()
        .into_iter()
        .chain(config.theme.warnings())
        .chain(config.changes.warnings())
        .chain(config.mermaid.style.warnings())
        .chain(edit_mode_warning)
        .collect();
    setting_warnings.iter().for_each(warn);
    let agent = config.theme.agent();
    let agent_highlighter =
        Highlighter::with_config(theme_config.clone().merge_cli(agent.code.as_deref(), false))
            .unwrap_or_else(|e| {
                warn(format!("Failed to load agent theme: {e}"));
                Highlighter::new()
            });
    let agent_theme = MarkdownTheme::new(agent.palette(ThemeRole::Agent).0);
    let prompt_files = chat_config.prompt_files(dirs::home_dir().as_deref());
    let (custom_prompt, prompt_warnings) = load_custom_prompt(&prompt_files);
    prompt_warnings.iter().for_each(warn);
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
    let (gate, sink) = prompt::turn_output(&theme_config)?;
    let turn = Turn {
        store: &store,
        store_dir: &store_dir,
        fork: fork.as_ref(),
        mcp_args: &mcp_args,
        system_prompt_file: &system_prompt_file,
        permission_mode: &permission_mode,
        plugin_args: &plugin_args,
        model_args: &model_args,
        skills: &skills,
        custom_prompt: &custom_prompt,
        reply_images,
        isolation: &isolation,
        session: Arc::new(Mutex::new(None)),
        process_turns: AtomicU64::new(0),
        reset: Mutex::new(None),
        live: Arc::clone(&live),
        reset_limits,
        recent_history_tokens,
        gate,
        run: &run,
        suggestions: suggestions.clone(),
        last_turn_end: Arc::clone(&last_turn_end),
        identity: Arc::default(),
        last_cost_usd: Mutex::new(None),
        timing: Mutex::default(),
        seen_len: Mutex::new(None),
        sink,
        show_tools: io::stdout().is_terminal(),
        terminated: signal::is_terminated,
        exe: Some(&exe),
    };
    let snapshots = TurnSnapshots::open(
        snapshots_enabled(Some(config.changes.enabled)),
        &store_dir,
        &std::env::current_dir()?,
    );
    let inbox = Arc::new(Inbox::new(store_dir.join(INBOX_DIR)));
    let known = inbox.peek().unwrap_or_default();
    let usage_stamp = memory::usage_stamp(&memory::usage_path()).unwrap_or_default();
    let terminal_status = status_enabled(
        chat_config.terminal_status,
        std::io::stdout().is_terminal(),
        std::env::var_os(REX_SESSION_ENV).is_some_and(|value| !value.is_empty()),
    );
    let monitor = Arc::new(Monitor {
        core: Core {
            watches: Watches::new(Arc::clone(&inbox), &store_dir, watch_timeout),
            schedule: ScheduleStore::new(store_dir.join(SCHEDULE_DIR)),
            inbox,
            store: Store::new(store_dir.clone()),
            store_dir: store_root.clone(),
            status: Mutex::new(Status::default()),
            seen: Mutex::new(HashMap::new()),
            active: Mutex::new(None),
            recent_history_tokens,
            worker_check_secs,
            worker_quiet_secs,
            auto_continue: chat_config.auto_continue,
            hold_progress: chat_config.hold_progress,
            workers: Workers::new(
                store_dir.clone(),
                exe.clone(),
                permission_mode.clone(),
                args.worktree_dir.clone(),
            ),
            check: Mutex::new(CheckState::seeded(&store)),
            known: Mutex::new(known),
            fresh: Mutex::new((Vec::new(), 0)),
            starting: Arc::default(),
            scheduled_reports: Mutex::new(Vec::new()),
            warned: Mutex::new(HashSet::new()),
            unrecorded: Mutex::new(HashSet::new()),
            live,
            last_ping: Arc::clone(&last_ping),
            run: run.clone(),
            reset_limits,
            process: Mutex::new(ProcessMemo::default()),
            usage_tail: Mutex::new(UsageTail::starting_at(&usage_stamp)),
            phase: Mutex::new(Phase::Prompt),
            hold: Mutex::new(None),
        },
        alert: AlertConfig {
            sound: chat_config.inbox_alert_sound && !args.no_inbox_sound,
            notification: chat_config.inbox_alert_notification && !args.no_inbox_notification,
            unknown_focus: chat_config.inbox_alert_unknown_focus,
            terminal_status,
        },
        term: Arc::new(TermStatus::new(
            terminal_status,
            chat_config.terminal_title && std::io::stdout().is_terminal(),
        )),
    });
    let session_name = name::session_name(
        args.name.as_deref(),
        chat_config.name.as_deref(),
        &cwd,
        name::resolved_home().as_deref(),
    );
    let thread_name = fork.as_ref().map(|fork| fork.name.as_str());
    monitor.term.start_title(&ralph_core::chat_name::title_name(
        &session_name,
        thread_name,
    ));
    let _term_clear = ClearOnDrop(Arc::clone(&monitor.term));
    let roster = {
        let monitor = Arc::clone(&monitor);
        let store = Store::new(store_dir.clone());
        move || {
            tool_report(
                &store,
                &running_workers(&monitor.store_dir),
                &monitor.status().workers,
                monitor.inbox.peek().ok().as_deref(),
                chrono::Utc::now(),
            )
        }
    };
    let (send_it_limits, send_it_warnings) = chat_config.send_it_setting();
    let send_it = SendIt::new(&store_dir, send_it_limits);
    let _close = monitor.watches.close_guard();
    monitor.watches.restore();
    chat_socket.serve(
        Shared {
            store: Store::new(store_dir.clone()),
            workers: Workers::new(
                store_dir.clone(),
                exe.clone(),
                permission_mode.clone(),
                args.worktree_dir.clone(),
            ),
            send_it: send_it.clone(),
            watches: monitor.watches.clone(),
        },
        roster,
    );
    restore_schedule(
        &monitor.schedule,
        &ralph_server::chat::schedule_worker::alive_worker_ids(&monitor.store_dir),
    );
    monitor.refresh(true);
    let running = running_workers(&monitor.store_dir);
    if let Some(hint) = orphan_hint(
        &ready_ids(&monitor.store_dir),
        &running,
        &acp_alive_ids(&monitor.store_dir),
        &resumable_ids(&monitor.store_dir),
    ) {
        println!("{hint}");
    }
    let sent_style = sent::SentStyle::new(
        color.then(|| crate::input_theme::to_reedline(theme_config.ui().ask_header)),
    );
    if chat_config.resume_summary && !args.skip_summary {
        let width = prompt::screen_columns();
        recap::print_recap(&store, running.len(), sent_style, width, &mut io::stdout())?;
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
            store: usage_store(&store_dir),
        }
        .spawn();
    }
    let headless = watch_headless(monitor.store_dir.clone());
    watch_input(
        Arc::clone(&at_prompt),
        Arc::clone(&turn.session),
        plugin.as_ref().map(|plugin| plugin.path().to_path_buf()),
        ExitCleanup::new(monitor.watches.clone(), &headless, socket_files),
        Arc::clone(&monitor.term),
    );
    let highlighter = suggestion_color(&config.theme.input.resolve().0, color)
        .map(|suggestion| SuggestionHighlighter::new(shown.clone(), suggestion));
    let printer = ExternalPrinter::new(PROMPT_ROWS);
    let prompt_sink = printer.sender();
    let mut editor = build_editor(&store_dir, edit_mode, highlighter, Arc::clone(&monitor))?
        .with_submit_highlighter(Box::new(sent_style))
        .with_wrap_width(wrap_cap.map(|cap| u16::try_from(cap).unwrap_or(u16::MAX)))
        .with_external_printer(printer);
    let repaint = editor.repaint_signal();
    watch_status(
        Arc::clone(&monitor),
        client_sink(Arc::clone(&monitor), move || repaint.request_repaint()),
    );
    crate::render::image::enable_images(image_settings);
    let (configured_editor, config_warning) = chat_config.editor_setting();
    let (editor_cmd, editor_warning) = editor_argv(
        configured_editor,
        std::env::var("VISUAL").ok().as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
    );
    let (empty_submit, empty_warning) = chat_config.empty_submit_setting();
    let late_warnings: Vec<String> = config_warning
        .into_iter()
        .chain(editor_warning)
        .chain(quiet_warning)
        .chain(reset_warnings)
        .chain(send_it_warnings)
        .chain(empty_warning)
        .collect();
    late_warnings.iter().for_each(warn);
    let config_snapshot = ConfigSnapshot {
        warnings: [
            loaded.warnings,
            setting_warnings,
            prompt_warnings,
            late_warnings,
        ]
        .concat(),
        global: FileState::of(global_config),
        store_file: FileState::of(store_config),
        store_dir: store_root,
        usage: FileState::of(absolute_from(&memory::usage_path(), &cwd)),
        claude_dir: dirs::home_dir()
            .map(|home| claude_dir(config.claude.config_dir.as_deref(), &home)),
        prompt_file: prompt_files.behaviour.map(FileState::of),
        append_prompts: prompt_files
            .appends
            .into_iter()
            .map(FileState::of)
            .collect(),
        skills: skill_dirs
            .iter()
            .map(|dir| FileState::of(absolute_from(dir, &cwd)))
            .collect(),
        env: config::chat_env(),
        flags,
        rows: loaded.rows,
        effective: loaded.effective,
    };
    let prompt = ChatPrompt {
        monitor: Arc::clone(&monitor),
        color,
        bar: StatusBar::new(
            &config.theme.status,
            &chat_config,
            color,
            &BarContext {
                name: &session_name,
                thread: thread_name,
                model_args: &model_args,
                identity: Arc::clone(&turn.identity),
            },
        ),
        notice: RefCell::new(None),
        editor: editor_cmd,
        empty_submit,
        sent_style,
    };
    let inbox_display = |color| Display {
        ctx: RenderContext::new(&agent_highlighter, color)
            .with_diff(diff_colors)
            .with_tools(tool_colors),
        theme: &agent_theme,
        color,
        ui: *theme_config.ui(),
    };
    let remote = RemoteStatus::default();
    let mut shell_state = cell::ShellState::default();
    let display = inbox_display(color);
    let local_route = LocalRoute {
        inbox: &monitor.inbox,
        watches: &monitor.watches,
        schedule: &monitor.schedule,
        display: &display,
        changes: &config.changes,
        prices: &config.wire.prices,
        tickets: &config.tickets,
        store: &store_dir,
        monitor: &monitor,
        remote: &remote,
        config: &config_snapshot,
        columns: prompt::screen_columns,
    };
    let keys = Keys {
        gate: &turn.gate,
        monitor: &monitor,
        bar: &prompt.bar,
        edit_mode,
        local: &local_route,
        empty_submit: &prompt.empty_submit,
        store: &store_dir,
        editor: &prompt.editor,
    };
    let inbox_turn = InboxTurn {
        turn: &turn,
        snapshots: &snapshots,
        keys: &keys,
        monitor: &monitor,
    };
    let schedule_store = monitor.schedule.clone();
    approve_proposed(&schedule_store, watch_timeout);
    let mut pending: Option<Submitted> = None;
    let mut shown_plan: Option<String> = None;
    let mut sending: Option<Active> = None;
    let mut offered: Option<(String, u64)> = None;
    let mut carried: Vec<String> = Vec::new();
    let mut leaving: Option<ChatExit> = None;
    let settles_closes = fork.is_none();
    if settles_closes && thread::settle::scan(&store_dir, &monitor.inbox) {
        monitor.refresh(true);
    }
    loop {
        warn_if_err(editor.sync_history(), "Failed to save chat history");
        let typed_at_prompt = pending.is_none();
        let submitted = match pending.take() {
            Some(submitted) => Some(submitted),
            None => {
                at_prompt.store(true, Ordering::SeqCst);
                idle.store(true, Ordering::SeqCst);
                print_above_prompt(Some(prompt_sink.clone()));
                let submitted = next_route(&mut editor, &prompt);
                print_above_prompt(None);
                at_prompt.store(false, Ordering::SeqCst);
                idle.store(false, Ordering::SeqCst);
                monitor.set_phase(Phase::Prompt);
                submitted?
                    .map_err(|exit| {
                        if matches!(exit, ChatExit::Closed) {
                            leaving = Some(chat_exit(LoopExit::IdleClose));
                        }
                    })
                    .ok()
                    .inspect(|submitted| {
                        if let Some((suggestion, turn_number)) = offered.take() {
                            let record = SuggestionRecord::new(
                                chrono::Utc::now().to_rfc3339(),
                                &run,
                                turn_number,
                                &suggestion,
                                &submitted.line,
                            );
                            warn_if_err(
                                store.append_suggestion(&record),
                                "Failed to record suggestion outcome",
                            );
                        }
                    })
            }
        };
        let Some(Submitted {
            line: typed_line,
            route,
        }) = submitted
        else {
            break;
        };
        let asked = settles_closes
            && typed_at_prompt
            && typed_line.trim().is_empty()
            && thread::settle::scan(&store_dir, &monitor.inbox);
        if asked {
            monitor.refresh(true);
            if !(matches!(route, Route::ReadInbox(_)) && monitor.status().unread > 0) {
                continue;
            }
        }
        let record = CommandRecord::for_route(&route, &typed_line);
        let note = |output: &str| {
            record
                .row(output)
                .and_then(|row| thread_cmd::append_row(&store, &row))
        };
        let mut queue = Queue::default();
        let mut drained = Vec::new();
        let (outcome, draft) = match route {
            Route::Ignore => continue,
            Route::Shell(line) => {
                monitor.set_phase(Phase::Cell);
                let ran = cell::run_local(&line, &mut shell_state, color);
                monitor.set_phase(Phase::Prompt);
                monitor.refresh(true);
                match ran {
                    Ok(shown) => {
                        cell::show(&mut std::io::stdout(), &display, &shown);
                        if let Some((text, rest)) = next_batch(std::mem::take(&mut carried)) {
                            carried = rest;
                            pending = Some(route_queued(text, &mut editor, &prompt));
                        }
                    }
                    Err(TurnOutcome::Exit) => {
                        let lost = unsent_texts(&carried, editor.current_buffer_contents());
                        let saved = save_unsent(&mut editor, &lost);
                        print_exit_notice(lost.len(), saved);
                        editor.sync_history()?;
                        leaving = Some(chat_exit(LoopExit::ShellCell));
                        break;
                    }
                    Err(_) => restore_prompt(&mut editor, &line, std::mem::take(&mut carried)),
                }
                continue;
            }
            Route::Send(message) => {
                crate::render::image::print_attachments(&message);
                monitor.set_phase(Phase::Cell);
                let outcome = match cell::prepare(&message, cell_limits, &mut shell_state, color) {
                    Ok(prepared) => {
                        if let Some(shown) = &prepared.shown {
                            cell::show(&mut std::io::stdout(), &display, shown);
                        }
                        let sent = prepared.sent;
                        monitor.set_phase(Phase::Turn);
                        let trigger = if sending.is_some() {
                            Trigger::SendIt
                        } else {
                            Trigger::User
                        };
                        let (result, queued) = snapshots.around(|| {
                            keys.during(|cancel| {
                                turn.run(trigger, std::slice::from_ref(&sent), &sent, cancel)
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
                let display = inbox_display(color);
                let Some(read) = inbox_turn.read(&send_it, selection, &display, &mut sending)?
                else {
                    continue;
                };
                (queue, drained) = (read.queue, read.drained);
                (read.outcome, None)
            }
            Route::Hold(filter) => {
                idle.store(true, Ordering::SeqCst);
                monitor.set_phase(Phase::Turn);
                let held = hold::run(&filter, &monitor, &prompt.bar);
                idle.store(false, Ordering::SeqCst);
                match held {
                    Held::Item(position) => {
                        let driven = send_it.held_item(&mut sending, position, &monitor)?;
                        pending = driven.enter(&mut editor);
                    }
                    Held::Left => {
                        monitor.set_phase(Phase::Prompt);
                        if let Some(active) = sending.take() {
                            send_it.took_over(&active, &monitor).enter(&mut editor);
                        }
                    }
                    Held::Quit => {
                        leaving = Some(chat_exit(LoopExit::HeldQuit));
                        break;
                    }
                }
                continue;
            }
            Route::SendIt(SendItCommand::Propose(text)) => {
                if let Some((ticket, name)) = send_it.template_use(&text) {
                    let screen = send_it.use_template(&ticket, &name);
                    println!("{}", screen.for_terminal(color));
                    note(&screen.text);
                    shown_plan = screen.shown;
                    continue;
                }
                if let Some(refusal) = send_it.refusal() {
                    println!("{refusal}");
                    note(&refusal);
                    continue;
                }
                let before = send_it.current_hash();
                let sent = propose_prompt(&text);
                monitor.set_phase(Phase::Turn);
                let (result, queued) = snapshots.around(|| {
                    keys.during(|cancel| {
                        turn.run(Trigger::User, std::slice::from_ref(&sent), &sent, cancel)
                    })
                });
                queue = queued;
                let outcome = result?;
                let screen = send_it.proposed(before);
                println!("{}", screen.for_terminal(color));
                shown_plan = screen.shown;
                (outcome, None)
            }
            Route::SendIt(SendItCommand::Save { name, ticket }) => {
                let saved = send_it.save_template(&name, ticket.as_deref());
                println!("{saved}");
                note(&saved);
                continue;
            }
            Route::SendIt(SendItCommand::Templates) => {
                println!("{}", send_it.templates());
                continue;
            }
            Route::SendIt(SendItCommand::Usage) => {
                println!("{}", send_it_usage(&send_it_limits));
                continue;
            }
            Route::SendIt(SendItCommand::Go) => {
                let (last, running) = worker_states(&monitor);
                let seen = Seen {
                    last: &last,
                    running: &running,
                };
                let (text, started) = send_it.go(shown_plan.as_deref(), typed_at_prompt, &seen);
                println!("{text}");
                note(&text);
                if let Some(started) = started {
                    shown_plan = None;
                    let mut driven = send_it.dispatch(started, &monitor);
                    (sending, pending) = (driven.active.take(), driven.enter(&mut editor));
                }
                continue;
            }
            Route::SendIt(SendItCommand::Off) => {
                shown_plan = None;
                let off = send_it.off();
                println!("{off}");
                note(&off);
                continue;
            }
            Route::SendIt(command @ (SendItCommand::Status | SendItCommand::Edit)) => {
                let screen = if command == SendItCommand::Edit {
                    send_it.edit(&prompt.editor)
                } else {
                    send_it.status()
                };
                println!("{}", screen.for_terminal(color));
                note(&screen.text);
                shown_plan = screen.shown;
                continue;
            }
            Route::Switch { parsed, line } => {
                let leave = Leave {
                    store_dir: &store_dir,
                    current: args.thread.as_ref(),
                    monitor: &monitor,
                };
                let target = match parsed
                    .and_then(|target| check_switch(&leave, &target).map(|()| target))
                {
                    Ok(target) => target,
                    Err(refusal) => {
                        println!("{refusal}");
                        if refuse_or_exit(&mut editor, &line, std::mem::take(&mut carried))? {
                            leaving = Some(chat_exit(LoopExit::SwitchRefused));
                            break;
                        }
                        continue;
                    }
                };
                let lost = unsent_texts(&carried, editor.current_buffer_contents());
                let saved = save_unsent(&mut editor, &lost);
                if !saved {
                    if signal::is_terminated() {
                        print_exit_notice(lost.len(), saved);
                        eprintln!("could not save the unsent text; exiting on signal.");
                        leaving = Some(chat_exit(LoopExit::SwitchSaveFailed));
                        break;
                    }
                    eprintln!("could not save the unsent text; the switch is cancelled.");
                    restore_prompt(&mut editor, &line, std::mem::take(&mut carried));
                    continue;
                }
                print_exit_notice(lost.len(), saved);
                warn_if_err(editor.sync_history(), "Failed to save chat history");
                idle.store(false, Ordering::SeqCst);
                *turn.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
                leaving = Some(ChatExit::Switch(
                    target,
                    SwitchArgv {
                        store: thread::config_root(&store_dir),
                        recent_history_tokens: args.recent_history_tokens,
                        wrap_width: args.wrap_width,
                        skills: args.skills.clone(),
                        permission_mode: args.permission_mode.clone(),
                        worktree_dir: args.worktree_dir.clone(),
                        no_inbox_sound: args.no_inbox_sound,
                        no_inbox_notification: args.no_inbox_notification,
                        skip_summary: args.skip_summary,
                    },
                ));
                break;
            }
            Route::ThreadOp { op, line } => {
                let merged = thread_cmd::merging(&op);
                match op.and_then(|op| {
                    let env = thread_cmd::ThreadEnv {
                        store_dir: &store_dir,
                        in_thread: fork.is_some(),
                        width: prompt::screen_columns(),
                        display: &display,
                    };
                    thread_cmd::run(&op, &env, &mut thread_cmd::confirm, &|name| {
                        thread::running_ids(&thread::config_root(&store_dir), name)
                    })
                }) {
                    Ok(text) => {
                        println!("{text}");
                        let id = note(&text);
                        thread_cmd::land(&thread::config_root(&store_dir), merged, id);
                        monitor.refresh(true);
                        if let Some((text, rest)) = next_batch(std::mem::take(&mut carried)) {
                            carried = rest;
                            pending = Some(route_queued(text, &mut editor, &prompt));
                        }
                    }
                    Err(refusal) => {
                        println!("{refusal}");
                        note(&refusal);
                        if refuse_or_exit(&mut editor, &line, std::mem::take(&mut carried))? {
                            leaving = Some(chat_exit(LoopExit::ThreadOpRefused));
                            break;
                        }
                    }
                }
                continue;
            }
            local => {
                if let Some(output) = run_local(local, &local_route, &mut io::stdout(), || {
                    monitor.refresh(true);
                })? {
                    note(&output);
                }
                continue;
            }
        };
        queue.entries.splice(0..0, std::mem::take(&mut carried));
        monitor.set_phase(if outcome == TurnOutcome::Replied {
            Phase::Replied
        } else {
            Phase::Prompt
        });
        monitor.refresh(true);
        if outcome == TurnOutcome::Exit {
            let lost = unsent(&queue);
            let saved = save_unsent(&mut editor, &lost);
            print_exit_notice(lost.len(), saved);
            editor.sync_history()?;
            leaving = Some(chat_exit(LoopExit::TurnExit));
            break;
        }
        let asked = approve_proposed(&schedule_store, watch_timeout);
        if let Some(active) = sending.take() {
            let unread = monitor.inbox.peek().unwrap_or_default();
            let ended = Ended {
                file: None,
                replied: outcome == TurnOutcome::Replied,
                asked,
                typed: queue.held || !queue.typed.is_empty(),
                queued: &queue.entries,
                cost_usd: turn.take_cost().unwrap_or_default(),
                step_result: None,
                drained: &drained,
                unread: &unread,
                answered: &[],
                watched: None,
                refused: None,
            };
            let driven = send_it.dispatch(send_it.tick(&active, ended), &monitor);
            sending = driven.active;
            if driven.route.is_some() {
                pending = driven.route.map(Submitted::internal);
                continue;
            }
            suggestions.clear();
            if let Some(line) = driven.prefill {
                suggestions.set(line);
            }
        }
        let suggestion = suggestions.take();
        let typed = std::mem::take(&mut queue.typed);
        let (draft, suggestion) = match decide(outcome, queue) {
            Decision::Send { text, rest } => {
                carried = rest;
                pending = Some(route_queued(text, &mut editor, &prompt));
                (draft, None)
            }
            Decision::Keep(entries) => (draft_with_queue(draft, &entries), suggestion),
        };
        let draft = draft_with_typed(draft, &typed);
        let replied = outcome == TurnOutcome::Replied && !asked;
        shown.clear();
        offered = None;
        let turn_number = turn.process_turns.load(Ordering::SeqCst);
        let suggested = suggestion.clone();
        let next = turn_end(TurnEnd {
            auto_hold: chat_config.auto_hold,
            replied,
            draft,
            suggestion,
            status: &monitor.status(),
        });
        match next {
            Next::Hold(filter) => {
                print!("{}", prompt.dim(&auto_hold_notice(&filter)));
                if let Some(suggested) = suggested {
                    warn_if_err(
                        store.append_suggestion(&SuggestionRecord::held(
                            chrono::Utc::now().to_rfc3339(),
                            &run,
                            turn_number,
                            &suggested,
                        )),
                        "Failed to record suggestion outcome",
                    );
                }
                pending = Some(Submitted::internal(Route::Hold(filter)));
            }
            Next::Prefill(fill) => {
                let (Prefill::Draft(text) | Prefill::Suggestion(text)) = &fill;
                editor.run_edit_commands(&[EditCommand::InsertString(text.clone())]);
                if let Prefill::Suggestion(text) = fill {
                    offered = Some((text.clone(), turn_number));
                    shown.set(text);
                }
            }
            Next::Nothing => {}
        }
    }
    Ok(leaving.unwrap_or(ChatExit::Quit))
}

#[cfg(test)]
mod tests;
