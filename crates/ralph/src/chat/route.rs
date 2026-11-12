use crate::chat::config;
use crate::chat::diff::{self, DiffContext};
use crate::chat::direct;
use crate::chat::inbox::{self, Display};
use crate::chat::monitor::Monitor;
use crate::chat::nudge;
use crate::chat::prompt::screen_columns;
use crate::chat::rerender;
use crate::chat::schedule;
use crate::chat::thread::{self, Memory};
use crate::chat::view;
use crate::chat::workers;
use crate::config::ConfigSnapshot;
use crate::schedule_store::ScheduleStore;
use ralph_core::chat_help::help_with_store;
use ralph_core::chat_status::Route;
use ralph_core::diff_command::{DiffArgs, UNCLOSED_DIFF_QUOTE};
use ralph_core::wire::price::PriceOverride;
use ralph_core::worker::TicketKeys;
use ralph_server::chat::watch;
use ralph_server::chat::workers_remote::RemoteStatus;
use ralph_server::config::ChangesSection;
use ralph_server::watches::Watches;
use ralph_server::worker_mcp::Inbox;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

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
}

pub fn run_local(route: Route, ctx: &LocalRoute, refresh: impl Fn()) -> io::Result<Option<String>> {
    let output = match route {
        Route::DropInbox(id) => inbox::drop_report(ctx.inbox, id)?,
        Route::Watch(command) => watch::run(command, ctx.watches),
        Route::Schedule(command) => {
            schedule::run(command, ctx.schedule, ctx.watches, &ctx.monitor.workers)
        }
        Route::Nudge(command) => nudge::run(command, ctx.monitor),
        Route::Fork(command) => thread::fork(ctx.store, command),
        other => {
            run_shown(other, ctx)?;
            return Ok(None);
        }
    };
    println!("{output}");
    refresh();
    Ok(Some(output))
}

fn run_shown(route: Route, ctx: &LocalRoute) -> io::Result<()> {
    match route {
        Route::ListInbox => inbox::list(ctx.inbox)?,
        Route::PeekInbox(id) => inbox::peek(ctx.inbox, id, ctx.display)?,
        Route::InboxUsage => inbox::usage(),
        Route::Diff(DiffArgs::Target(target)) => diff::run(
            &DiffContext {
                changes: ctx.changes,
                store: ctx.store,
            },
            target,
        ),
        Route::Diff(DiffArgs::Usage(usage)) => println!("{usage}"),
        Route::Diff(DiffArgs::UnclosedQuote) => println!("{UNCLOSED_DIFF_QUOTE}"),
        Route::Workers(id) => workers::run(ctx.monitor, id.as_deref(), ctx.display, ctx.remote),
        Route::Direct(direct) => direct::run(direct, ctx),
        Route::WorkerCosts(args) => {
            workers::costs(ctx.display, ctx.prices, ctx.tickets, ctx.store, args)
        }
        Route::Help => print!("{}", help_with_store(screen_columns(), ctx.store)),
        Route::Config => config::print(ctx.config),
        Route::View(args) => view::run(&args, ctx.display),
        Route::Rerender(n) => rerender::run(&n, ctx.store, ctx.display),
        Route::History(call) => println!(
            "{}",
            call.and_then(|call| Memory::open(ctx.store)?.call(&call))
                .unwrap_or_else(|e| e)
        ),
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
        | Route::Fork(_)
        | Route::Ignore => {}
    }
    Ok(())
}
