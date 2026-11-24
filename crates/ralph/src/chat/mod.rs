pub(crate) mod assemble;
mod bounded;
pub(crate) mod cell;
pub(crate) mod client_monitor;
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
mod nudge;
mod prompt;
mod queue;
mod recap;
mod refusal;
mod remote;
mod rerender;
mod resize;
mod route;
mod schedule;
pub(crate) use ralph_server::chat::tag_store;
mod send_it;
mod sent;
pub(crate) mod session;
mod statusbar;
mod suggestion;
mod switch;
mod term_loop;
mod term_status;
pub(crate) mod terminal;
pub(crate) mod thread;
pub(crate) mod thread_cmd;
mod view;
mod workers;

use crate::chat::assemble::{assemble, load_settings, AssembleOptions, Assembled, TurnOutput};
use crate::chat::client_monitor::{client_events, client_sink, watch_headless, watch_input};
use crate::chat::editor::build_editor;
use crate::chat::inbox::Display;
use crate::chat::keys::Keys;
use crate::chat::prompt::ChatPrompt;
use crate::chat::resize::{Resize, Watch};
use crate::chat::route::LocalRoute;
use crate::chat::schedule::approve_proposed;
use crate::chat::send_it::InboxTurn;
use crate::chat::statusbar::{BarContext, StatusBar};
use crate::chat::suggestion::SuggestionHighlighter;
use crate::chat::switch::chat_exit;
use crate::chat::term_loop::TerminalLoop;
use crate::chat::term_status::{ClearOnDrop, TermStatus};
use crate::cli::{ChatArgs, ChatCommand};
use crate::config::{ConfigSnapshot, FileState, MermaidStyleExt, ThemeWarnings};
use crate::highlight::{Highlighter, ThemeConfig};
use crate::input_theme::suggestion_color;
use crate::markdown::{MarkdownTheme, RolePalette, ThemeRole};
use crate::render::{cap_wrap_width, RenderContext};
use crate::signal;
use crate::theme_slots::ResolveColors;
use ralph_core::chat::{resolve_wrap_width, ReplyImages};
use ralph_core::chat_cell::resolve_cell_limits;
use ralph_core::chat_close::LoopExit;
use ralph_core::chat_draft::editor_argv;
use ralph_core::chat_edit_mode::resolve_edit_mode;
use ralph_core::chat_memory::store_in_use_message;
use ralph_core::chat_status::orphan_hint;
use ralph_core::chat_switch::SwitchArgv;
use ralph_core::claude_leftovers::claude_dir;
use ralph_core::inbox_alert::AlertConfig;
use ralph_core::program_status::status_enabled;
use ralph_server::chat::host::spawn_session_host;
use ralph_server::chat::memory::Store;
use ralph_server::chat::monitor::ready_ids;
use ralph_server::chat::suggestion::SuggestionCell;
use ralph_server::chat::workers_remote::RemoteStatus;
use ralph_server::chat::{memory, name};
use ralph_server::cursor_owner::{acp_alive_ids, resumable_ids};
use ralph_server::paths::absolute_from;
use ralph_server::warn::{warn, warn_if_err};
use ralph_server::workers::headless::ExitCleanup;
use ralph_server::workers::{canonical_store, running_workers};
use reedline::ExternalPrinter;
#[cfg(test)]
use session::resolve_store;
use session::{chat_dir, load_custom_prompt, print_history, store_dir};
use std::cell::RefCell;
use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
pub(crate) use switch::ChatExit;

pub(crate) const STORE_ENV: &str = "RALPH_CHAT_STORE";
const REX_SESSION_ENV: &str = "REX_SESSION";
const PROMPT_ROWS: usize = 256;
pub(crate) const WRAP_WIDTH_ENV: &str = "RALPH_WRAP_WIDTH";
pub(crate) const CELL_OUTPUT_LIMIT_ENV: &str = "RALPH_CELL_OUTPUT_LIMIT";
pub(crate) const CELL_OUTPUT_BYTES_ENV: &str = "RALPH_CELL_OUTPUT_BYTES";

pub fn run(args: ChatArgs) -> Result<ChatExit, Box<dyn std::error::Error>> {
    if let Some(socket) = &args.connect {
        return remote::run(socket);
    }
    if let Some(ChatCommand::History(history)) = args.command {
        print_history(&chat_dir(args.store, args.thread.as_ref())?, &history)?;
        return Ok(chat_exit(LoopExit::History));
    }
    warn_if_err(signal::init(), "Failed to initialize signal handler");
    signal::ignore_sigttou();
    let flags = config::flag_settings(&args);
    let store_dir = chat_dir(args.store, args.thread.as_ref())?;
    std::fs::create_dir_all(&store_dir)?;
    let store_root = canonical_store(store_dir.clone());
    ralph_server::config::set_store_layer(&store_dir);
    let store = Store::new(store_dir.clone());
    let _chat = store
        .claim_chat()?
        .map_err(|holder| store_in_use_message(&store_root, holder))?;
    let model_args =
        thread::open::model_args(&store_dir, args.thread.as_ref(), args.model.as_deref())?;
    let fork = thread::open_fork(&store_dir)?;
    let settings = load_settings(
        &store_dir,
        args.permission_mode.as_deref(),
        args.recent_history_tokens,
    )?;
    let config = &settings.loaded.config;
    let tuning = &settings.tuning;
    let cwd = settings.cwd.clone();
    let wrap_cap = resolve_wrap_width(
        args.wrap_width,
        std::env::var(WRAP_WIDTH_ENV).ok().as_deref(),
        config.chat.wrap_width,
    )?;
    cap_wrap_width(wrap_cap);
    let cell_limits = resolve_cell_limits(
        std::env::var(CELL_OUTPUT_LIMIT_ENV).ok().as_deref(),
        std::env::var(CELL_OUTPUT_BYTES_ENV).ok().as_deref(),
        config.chat.cell_output_limit,
        config.chat.cell_output_bytes,
    )?;
    let chat_config = &config.chat;
    let shown = SuggestionCell::default();
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
    let thread = fork.as_ref().map(|fork| fork.name.clone());
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let image_settings = crate::render::image::detect(color);
    let reply_images = if image_settings.is_some() {
        ReplyImages::Inline
    } else {
        ReplyImages::Text
    };
    let resize = Arc::new(Resize::default());
    let (gate, sink) = prompt::turn_output(&theme_config, Arc::clone(&resize))?;
    let terminal_status = status_enabled(
        chat_config.terminal_status,
        std::io::stdout().is_terminal(),
        std::env::var_os(REX_SESSION_ENV).is_some_and(|value| !value.is_empty()),
    );
    let alert = AlertConfig {
        sound: chat_config.inbox_alert_sound && !args.no_inbox_sound,
        notification: chat_config.inbox_alert_notification && !args.no_inbox_notification,
        unknown_focus: chat_config.inbox_alert_unknown_focus,
        terminal_status,
    };
    let term = Arc::new(TermStatus::new(
        terminal_status,
        chat_config.terminal_title && std::io::stdout().is_terminal(),
    ));
    let assembled = assemble(AssembleOptions {
        store_dir: &store_dir,
        store_root: store_root.clone(),
        settings: &settings,
        fork,
        model_args: model_args.clone(),
        skills: &args.skills,
        worktree_dir: args.worktree_dir.clone(),
        custom_prompt,
        output: TurnOutput {
            gate,
            sink,
            reply_images,
            show_tools: io::stdout().is_terminal(),
        },
        events: client_events(alert, Arc::clone(&term)),
    })?;
    let Assembled {
        session,
        guard,
        skill_dirs,
        send_it_warnings,
        close: _close,
    } = assembled;
    let monitor = Arc::clone(&session.core);
    let suggestions = session.turn.suggestions.clone();
    let run = session.turn.run.clone();
    let session_name = name::session_name(
        args.name.as_deref(),
        chat_config.name.as_deref(),
        &cwd,
        name::resolved_home().as_deref(),
    );
    let thread_name = thread.as_ref().map(|name| name.as_str());
    term.start_title(&ralph_core::chat_name::title_name(
        &session_name,
        thread_name,
    ));
    let _term_clear = ClearOnDrop(Arc::clone(&term));
    let turn = &session.turn;
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
    let headless = watch_headless(monitor.store_dir.clone());
    watch_input(
        Arc::clone(&at_prompt),
        Arc::clone(&turn.session),
        ExitCleanup::new(monitor.watches.clone(), &headless, guard.0.clone()),
        Arc::clone(&term),
    );
    let highlighter = suggestion_color(&config.theme.input.resolve().0, color)
        .map(|suggestion| SuggestionHighlighter::new(shown.clone(), suggestion, sent_style));
    let printer = ExternalPrinter::new(PROMPT_ROWS);
    let prompt_sink = printer.sender();
    let resize_sink = printer.sender();
    let mut editor = build_editor(&store_dir, edit_mode, highlighter, Arc::clone(&monitor))?
        .with_submit_highlighter(Box::new(sent_style))
        .with_wrap_width(wrap_cap.map(|cap| u16::try_from(cap).unwrap_or(u16::MAX)))
        .with_external_printer(printer);
    let repaint = editor.repaint_signal();
    let _host = spawn_session_host(
        &session,
        tuning.keepalive_hours,
        Arc::clone(&idle),
        true,
        client_sink(Arc::clone(&monitor), alert, Arc::clone(&term), move || {
            repaint.request_repaint()
        }),
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
        .chain(tuning.quiet_warning.clone())
        .chain(tuning.reset_warnings.clone())
        .chain(send_it_warnings)
        .chain(empty_warning)
        .collect();
    late_warnings.iter().for_each(warn);
    let config_snapshot = ConfigSnapshot {
        warnings: [
            settings.loaded.warnings,
            setting_warnings,
            prompt_warnings,
            late_warnings,
        ]
        .concat(),
        global: FileState::of(settings.global_config.clone()),
        store_file: FileState::of(settings.store_config.clone()),
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
        rows: settings.loaded.rows,
        effective: settings.loaded.effective,
    };
    let prompt = ChatPrompt {
        monitor: Arc::clone(&monitor),
        color,
        bar: StatusBar::new(
            &config.theme.status,
            chat_config,
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
    let display = inbox_display(color);
    let local_route = LocalRoute {
        gate: &turn.gate,
        inbox: &monitor.inbox,
        watches: &monitor.watches,
        schedule: &monitor.schedule,
        display: &display,
        changes: &config.changes,
        prices: &config.wire.prices,
        tickets: &config.tickets,
        store: &store_dir,
        thread: args.thread.as_ref(),
        monitor: &monitor,
        remote: &remote,
        config: &config_snapshot,
        columns: prompt::screen_columns,
        resize: &resize,
    };
    let keys = Keys {
        gate: &turn.gate,
        monitor: &monitor,
        bar: &prompt.bar,
        edit_mode,
        sent_style,
        local: &local_route,
        empty_submit: &prompt.empty_submit,
        store: &store_dir,
        editor: &prompt.editor,
    };
    let inbox_turn = InboxTurn {
        port: &session,
        keys: &keys,
        monitor: &monitor,
    };
    approve_proposed(&monitor.schedule, tuning.watch_timeout);
    let lp = TerminalLoop {
        session: &session,
        prompt: &prompt,
        store: &store,
        run: &run,
        suggestions: &suggestions,
        shown: &shown,
        watch_timeout: tuning.watch_timeout,
        auto_hold: chat_config.auto_hold,
        color,
        display: &display,
        idle: &idle,
        store_dir: &store_dir,
        thread: args.thread.as_ref(),
        switch_argv: SwitchArgv {
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
        shell: cell::ShellState::default(),
        keys: &keys,
        cell_limits,
        in_thread: thread.is_some(),
        shown_plan: None,
        at_prompt: &at_prompt,
        prompt_sink,
        inbox_turn: &inbox_turn,
        inbox_display: &inbox_display,
        local: &local_route,
        editor,
        pending: None,
        sending: None,
        offered: None,
        carried: Vec::new(),
        leaving: None,
        resize: &resize,
    };
    let resized = Arc::new(AtomicBool::new(false));
    let _winch = signal::arm_resize(&resized);
    let done = AtomicBool::new(false);
    let watch = Watch {
        resized: &resized,
        done: &done,
        at_prompt: &at_prompt,
        main_view: args.thread.is_none(),
        state: &resize,
        store: &store_dir,
        display: &display,
        sink: resize_sink,
    };
    std::thread::scope(|scope| {
        scope.spawn(|| watch.run());
        let exit = lp.run();
        done.store(true, Ordering::SeqCst);
        exit
    })
}

#[cfg(test)]
mod tests;
