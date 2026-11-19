use crate::chat::config;
use crate::chat::diff::{self, DiffContext};
use crate::chat::direct;
use crate::chat::inbox::{self, Display};
use crate::chat::monitor::Monitor;
use crate::chat::nudge;
use crate::chat::prompt::Columns;
use crate::chat::rerender;
use crate::chat::schedule;
use crate::chat::thread::Memory;
use crate::chat::view;
use crate::chat::workers;
use crate::config::ConfigSnapshot;
use crate::schedule_store::ScheduleStore;
use ralph_core::chat_help::help_with_store;
use ralph_core::chat_memory::{CallContext, CallMetrics, CallSource};
use ralph_core::chat_status::Route;
use ralph_core::diff_command::{DiffArgs, UNCLOSED_DIFF_QUOTE};
use ralph_core::wire::price::PriceOverride;
use ralph_core::worker::TicketKeys;
use ralph_server::chat::watch;
use ralph_server::chat::workers_remote::RemoteStatus;
use ralph_server::config::ChangesSection;
use ralph_server::history_log;
use ralph_server::watches::Watches;
use ralph_server::worker_mcp::Inbox;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

pub struct LocalRoute<'a> {
    pub inbox: &'a Inbox,
    pub watches: &'a Watches,
    pub schedule: &'a ScheduleStore,
    pub display: &'a Display<'a>,
    pub changes: &'a ChangesSection,
    pub prices: &'a BTreeMap<String, PriceOverride>,
    pub tickets: &'a TicketKeys,
    pub store: &'a Path,
    pub monitor: &'a Monitor,
    pub remote: &'a RemoteStatus,
    pub config: &'a ConfigSnapshot,
    pub columns: Columns,
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
        other => {
            run_shown(other, ctx, out)?;
            return Ok(None);
        }
    };
    writeln!(out, "{output}")?;
    refresh();
    Ok(Some(output))
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
        Route::Help => write!(out, "{}", help_with_store((ctx.columns)(), ctx.store))?,
        Route::Config => config::print(ctx.config),
        Route::View(args) => view::run(&args, ctx.display),
        Route::Rerender(n) => rerender::run(&n, ctx.store, ctx.display),
        Route::History(call) => {
            let text = Memory::open(ctx.store)
                .and_then(|memory| memory.call(&call, CallSource::Slash))
                .map_or_else(|e| e, |answer| answer.text);
            writeln!(out, "{text}")?
        }
        Route::HistoryRefused { tool, message } => {
            history_log::record_error(
                ctx.store,
                tool,
                &Value::Null,
                &message,
                CallContext {
                    source: CallSource::Slash,
                    took: Duration::ZERO,
                    metrics: CallMetrics::default(),
                },
            );
            writeln!(out, "{message}")?
        }
        Route::Send(_)
        | Route::Switch { .. }
        | Route::ThreadOp { .. }
        | Route::Shell(_)
        | Route::ReadInbox(_)
        | Route::Hold(_)
        | Route::SendIt(_)
        | Route::DropInbox(_)
        | Route::Watch(_)
        | Route::Schedule(_)
        | Route::Nudge(_)
        | Route::Ignore => {}
    }
    Ok(())
}
