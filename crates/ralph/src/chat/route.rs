use crate::chat::config;
use crate::chat::diff::{self, DiffContext};
use crate::chat::direct;
use crate::chat::inbox::{self, Display};
use crate::chat::nudge;
use crate::chat::prompt::Columns;
use crate::chat::rerender;
use crate::chat::resize::Resize;
use crate::chat::thread::{self, Memory};
use crate::chat::thread_cmd::{self, ThreadEnv};
use crate::chat::view;
use crate::chat::workers;
use crate::config::ConfigSnapshot;
use ralph_core::chat_help::help_with_store;
use ralph_core::chat_memory::{history_tool_of, CallSource, MemoryCall};
use ralph_core::chat_stage::view::{view_lines, StageView};
use ralph_core::chat_stage::{stage_text, StageCommand, STAGE_EMPTY_LINE, STAGE_USAGE};
use ralph_core::chat_status::Route;
use ralph_core::chat_thread::ThreadName;
use ralph_core::chat_thread_cmd::ThreadOp;
use ralph_core::diff_command::{DiffArgs, UNCLOSED_DIFF_QUOTE};
use ralph_core::wire::price::PriceOverride;
use ralph_core::worker::{TicketKeys, HANDOFF_TIMEOUT};
use ralph_server::chat::memory::{Answer, Store};
use ralph_server::chat::monitor::Monitor;
use ralph_server::chat::schedule;
use ralph_server::chat::stage;
use ralph_server::chat::watch;
use ralph_server::chat::workers_remote::RemoteStatus;
use ralph_server::config::ChangesSection;
use ralph_server::history_log;
use ralph_server::schedule_store::ScheduleStore;
use ralph_server::watches::Watches;
use ralph_server::worker_mcp::Inbox;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

pub struct LocalRoute<'a> {
    pub gate: &'a ralph_server::subprocess::OutputGate,
    pub inbox: &'a Inbox,
    pub watches: &'a Watches,
    pub schedule: &'a ScheduleStore,
    pub display: &'a Display<'a>,
    pub reply_display: &'a Display<'a>,
    pub changes: &'a ChangesSection,
    pub prices: &'a BTreeMap<String, PriceOverride>,
    pub tickets: &'a TicketKeys,
    pub store: &'a Path,
    pub thread: Option<&'a ThreadName>,
    pub monitor: &'a Monitor,
    pub remote: &'a RemoteStatus,
    pub config: &'a ConfigSnapshot,
    pub columns: Columns,
    pub resize: &'a Resize,
}

pub fn run_local(
    route: Route,
    ctx: &LocalRoute,
    out: &mut dyn Write,
    refresh: impl Fn(),
) -> io::Result<Option<String>> {
    let output = match route {
        Route::DropInbox(id) => inbox::drop_report(ctx.inbox, id)?,
        Route::Watch(command) => watch::run(command, ctx.watches),
        Route::Schedule(command) => {
            schedule::run(command, ctx.schedule, ctx.watches, &ctx.monitor.workers)
        }
        Route::Nudge(command) => nudge::run(command, ctx.monitor),
        Route::ThreadOp { op, .. } => run_thread(op, ctx),
        Route::Stage(command) => run_stage(command, ctx),
        other => {
            run_shown(other, ctx, out)?;
            return Ok(None);
        }
    };
    writeln!(out, "{output}")?;
    refresh();
    Ok(Some(output))
}

fn run_stage(command: StageCommand, ctx: &LocalRoute) -> String {
    let store = Store::new(ctx.store.to_path_buf());
    match command {
        StageCommand::Show => stage::view(&store, &ctx.monitor.status(), ctx.thread)
            .map(|view| stage_shown(view, (ctx.columns)(), ctx.display)),
        StageCommand::Raw => stage::rendered(ctx.store, &ctx.monitor.status(), ctx.thread)
            .map(|rendered| stage_text(&rendered).to_string()),
        StageCommand::Add(new) => stage::user_add(&store, new),
        StageCommand::Done(id) => stage::user_done(&store, &id),
        StageCommand::Edit { id, text } => stage::user_edit(&store, &id, &text),
        StageCommand::Usage => Ok(STAGE_USAGE.to_string()),
    }
    .unwrap_or_else(|text| text)
}

fn stage_shown(view: Option<StageView>, width: usize, display: &Display) -> String {
    let width = if display.color { width } else { usize::MAX };
    view.map_or_else(
        || STAGE_EMPTY_LINE.to_string(),
        |view| {
            workers::paint_lines(
                &view_lines(&view, width),
                &display.theme.palette,
                display.color,
            )
        },
    )
}

fn run_thread(op: Result<ThreadOp, String>, ctx: &LocalRoute) -> String {
    let main = thread::config_root(ctx.store);
    let env = ThreadEnv {
        store_dir: ctx.store,
        in_thread: ctx.thread.is_some(),
        mid_turn: true,
        width: (ctx.columns)(),
        display: Some(ctx.display),
        notice: &|_| {},
        curate: &|prompt, curator| crate::run_curator(prompt, curator),
    };
    op.and_then(|op| {
        thread_cmd::run(&op, &env, &mut |_| None, &|name| {
            thread::running_ids(&main, name)
        })
    })
    .unwrap_or_else(|text| text)
}

pub(super) fn history_call(
    store: &Path,
    call: &MemoryCall,
    source: CallSource,
) -> Result<Answer, String> {
    Memory::open(store)
        .inspect_err(|message| {
            history_log::record_refusal(store, history_tool_of(&call.tool), message, source);
        })?
        .call(call, source)
}

fn run_shown(route: Route, ctx: &LocalRoute, out: &mut dyn Write) -> io::Result<()> {
    match route {
        Route::ListInbox => inbox::list(ctx.inbox, out)?,
        Route::PeekInbox(id) => inbox::peek(ctx.inbox, id, ctx.display, (ctx.columns)(), out)?,
        Route::InboxUsage => inbox::usage(out)?,
        Route::Diff(DiffArgs::Target(target)) => diff::run(
            &DiffContext {
                changes: ctx.changes,
                store: ctx.store,
            },
            target,
        ),
        Route::Diff(DiffArgs::Usage(usage)) => writeln!(out, "{usage}")?,
        Route::Diff(DiffArgs::UnclosedQuote) => writeln!(out, "{UNCLOSED_DIFF_QUOTE}")?,
        Route::Workers(id) => workers::run(ctx.monitor, id.as_deref(), ctx.display, ctx.remote),
        Route::Direct(direct) => direct::run(direct, ctx),
        Route::WorkerCosts(args) => {
            workers::costs(ctx.display, ctx.prices, ctx.tickets, ctx.store, args)
        }
        Route::Answer(answer) => {
            let message = answer.and_then(|answer| {
                if answer.worker == "@chat" {
                    return Ok(ctx
                        .gate
                        .answer_permission(&answer.request, &answer.option)
                        .report(ctx.store));
                }
                ralph_server::workers::answer_permission(
                    ctx.store,
                    HANDOFF_TIMEOUT,
                    &answer.worker,
                    &answer.request,
                    &answer.option,
                )
                .map_err(|error| error.to_string())
            });
            writeln!(out, "{}", message.unwrap_or_else(|error| error))?;
        }
        Route::Help => write!(out, "{}", help_with_store((ctx.columns)(), ctx.store))?,
        Route::Config => config::print(ctx.config),
        Route::View(args) => view::run(&args, ctx.display),
        Route::Rerender(n) => rerender::run(&n, ctx.store, ctx.reply_display, ctx.resize),
        Route::History(call) => {
            let text = history_call(ctx.store, &call, CallSource::Slash)
                .map_or_else(|e| e, |answer| answer.text);
            writeln!(out, "{text}")?
        }
        Route::HistoryRefused { tool, message } => {
            history_log::record_refusal(ctx.store, tool, &message, CallSource::Slash);
            writeln!(out, "{message}")?
        }
        Route::Switch {
            parsed: Err(usage), ..
        } => writeln!(out, "{usage}")?,
        Route::Send(_)
        | Route::Switch { .. }
        | Route::ThreadOp { .. }
        | Route::Shell(_)
        | Route::ReadInbox(_)
        | Route::Hold(_)
        | Route::SendIt(_)
        | Route::Stage(_)
        | Route::DropInbox(_)
        | Route::Watch(_)
        | Route::Schedule(_)
        | Route::Nudge(_)
        | Route::Ignore => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::{fg, BOLD};
    use crate::chat::thread::thread_dir;
    use crate::highlight::Highlighter;
    use crate::markdown::MarkdownTheme;
    use crate::render::RenderContext;
    use crate::ui_theme::UiColors;
    use ralph_core::chat_memory::{history_path, parse_tool_call};
    use ralph_core::chat_stage::view::{stage_view, StageInput, StageView};
    use ralph_core::chat_stage::{Item, Kind};
    use ralph_core::chat_status::Status;
    use serde_json::{json, Value};

    fn owed_view() -> StageView {
        let items = [Item {
            id: "s1".to_string(),
            kind: Kind::Owed,
            reference: None,
            text: "send the note to the reviewer and link the screenshots".to_string(),
            created: "2026-01-01T00:00:00Z".to_string(),
            turn: 0,
        }];
        stage_view(&StageInput {
            items: &items,
            status: &Status::default(),
            thread: None,
            log_len: 0,
        })
        .unwrap()
    }

    fn shown_with(view: Option<StageView>, width: usize, color: bool) -> String {
        let highlighter = Highlighter::new();
        let theme = MarkdownTheme::default();
        let display = Display {
            ctx: RenderContext::new(&highlighter, color),
            theme: &theme,
            color,
            ui: UiColors::default(),
        };
        stage_shown(view, width, &display)
    }

    #[test]
    fn stage_show_prints_plain_unwrapped_lines_without_colour() {
        assert_eq!(
            shown_with(Some(owed_view()), 40, false),
            "Stage\n\nOpen\n  ● s1  owed      send the note to the reviewer and link the screenshots · ~30 turns left"
        );
    }

    #[test]
    fn stage_show_paints_tones_and_wraps_to_the_width_with_colour() {
        let text = shown_with(Some(owed_view()), 40, true);
        let palette = MarkdownTheme::default().palette;
        assert!(text.contains(&format!("{BOLD}Stage")), "{text:?}");
        assert!(text.contains(&format!("{}●", fg(palette.link))), "{text:?}");
        assert!(text.lines().count() > 4, "{text:?}");
    }

    #[test]
    fn an_empty_stage_still_prints_the_empty_line() {
        assert_eq!(shown_with(None, 80, true), "Stage: empty");
    }

    #[test]
    fn local_answer_routes_to_the_worker_handoff() {
        use crate::config::FileState;
        use crate::highlight::Highlighter;
        use crate::markdown::MarkdownTheme;
        use crate::render::RenderContext;
        use crate::ui_theme::UiColors;
        use ralph_core::chat_status::{route_input, EmptySubmit};
        use ralph_server::chat::keepalive::Stamp;
        use ralph_server::chat::session::{open_core, tuning, CoreParts};
        use ralph_server::config::AppConfig;
        use ralph_server::workers::Workers;
        use std::sync::{atomic::AtomicBool, Arc};

        let dir = tempfile::tempdir().unwrap();
        let config = AppConfig::default();
        let tuning = tuning(&config.chat, None).unwrap();
        let monitor = open_core(CoreParts {
            store_dir: dir.path(),
            store_root: dir.path().to_path_buf(),
            chat: &config.chat,
            tuning: &tuning,
            workers: Workers::new(
                dir.path().to_path_buf(),
                "ralph".into(),
                String::new(),
                None,
            ),
            live: Arc::new(AtomicBool::new(false)),
            last_ping: Stamp::default(),
            run: String::new(),
            caps: ralph_core::chat_backend::CLAUDE,
        });
        let highlighter = Highlighter::new();
        let theme = MarkdownTheme::default();
        let display = Display {
            ctx: RenderContext::new(&highlighter, false),
            theme: &theme,
            color: false,
            ui: UiColors::default(),
        };
        let file = FileState::of(dir.path().to_path_buf());
        let snapshot = ConfigSnapshot {
            warnings: vec![],
            global: file.clone(),
            store_file: file.clone(),
            store_dir: dir.path().to_path_buf(),
            usage: file,
            claude_dir: None,
            prompt_file: None,
            append_prompts: vec![],
            skills: vec![],
            env: vec![],
            flags: vec![],
            rows: vec![],
            effective: None,
        };
        let remote = RemoteStatus::default();
        let gate = ralph_server::subprocess::OutputGate::new();
        let ctx = LocalRoute {
            gate: &gate,
            inbox: &monitor.inbox,
            watches: &monitor.watches,
            schedule: &monitor.schedule,
            display: &display,
            reply_display: &display,
            changes: &config.changes,
            prices: &config.wire.prices,
            tickets: &config.tickets,
            store: dir.path(),
            thread: None,
            monitor: &monitor,
            remote: &remote,
            config: &snapshot,
            columns: || 80,
            resize: &Resize::default(),
        };
        let mut out = Vec::new();
        let route = route_input(
            "/answer missing req once",
            0,
            &EmptySubmit::Inbox,
            &Default::default(),
        );
        run_local(route, &ctx, &mut out, || {}).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "worker missing is not a running Cursor, Codex, claude or opencode worker\n"
        );
        let mut out = Vec::new();
        let route = route_input(
            "/answer missing req",
            0,
            &EmptySubmit::Inbox,
            &Default::default(),
        );
        run_local(route, &ctx, &mut out, || {}).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Usage: /answer <worker> <request> <option>\n"
        );

        let mut out = Vec::new();
        let route = route_input(
            "/answer @chat stale once",
            0,
            &EmptySubmit::Inbox,
            &Default::default(),
        );
        run_local(route, &ctx, &mut out, || {}).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "No matching main chat permission request.\n"
        );

        let files = ralph_core::worker::HandoffFiles::opencode(dir.path(), "A");
        ralph_server::cursor_owner::write_file(&files.owner, "").unwrap();
        let host = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if let Ok(body) = std::fs::read_to_string(&files.answer) {
                    std::fs::remove_file(&files.answer).unwrap();
                    return body;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        let mut out = Vec::new();
        let route = route_input(
            "/answer A req once",
            0,
            &EmptySubmit::Inbox,
            &Default::default(),
        );
        run_local(route, &ctx, &mut out, || {}).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Answered the permission request of worker A.\n"
        );
        let answer = ralph_core::worker::PermissionAnswer::decode(&host.join().unwrap()).unwrap();
        assert_eq!(
            (answer.request_id.as_str(), answer.option_id.as_str()),
            ("req", "once")
        );
    }

    fn recall() -> MemoryCall {
        parse_tool_call("recall", &json!({"pattern": "x"})).unwrap()
    }

    fn rows(store: &Path) -> Vec<Value> {
        std::fs::read_to_string(history_path(store))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn a_failed_open_of_an_existing_store_logs_an_error_row() {
        let root = tempfile::tempdir().unwrap();
        let auth = ThreadName::parse("auth").unwrap();
        let thread = thread_dir(&root.path().join("chat"), &auth);
        std::fs::create_dir_all(&thread).unwrap();
        let err = history_call(&thread, &recall(), CallSource::Slash).unwrap_err();
        let rows = rows(&thread);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["event"], "call");
        assert_eq!(rows[0]["tool"], "recall");
        assert_eq!(rows[0]["source"], "slash");
        assert_eq!(rows[0]["outcome"], "error");
        assert_eq!(rows[0]["error"], err);
    }

    #[test]
    fn a_failed_open_of_a_missing_store_logs_nothing_and_creates_nothing() {
        let root = tempfile::tempdir().unwrap();
        let auth = ThreadName::parse("auth").unwrap();
        let thread = thread_dir(&root.path().join("chat"), &auth);
        assert!(history_call(&thread, &recall(), CallSource::Cli).is_err());
        assert!(!thread.exists());
        assert!(!root.path().join("chat").exists());
    }
}
