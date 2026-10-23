use crate::chat::diff::{self, DiffContext};
use crate::chat::inbox::{self, Display};
use crate::chat::monitor::Monitor;
use crate::chat::prompt::screen_columns;
use crate::chat::watch;
use crate::chat::workers;
use crate::config::ChangesSection;
use crate::watches::Watches;
use crate::worker_mcp::Inbox;
use ralph_core::chat_help::help_text;
use ralph_core::chat_status::Route;
use ralph_core::diff_command::{DiffArgs, UNCLOSED_DIFF_QUOTE};
use std::io;
use std::path::Path;

pub struct LocalRoute<'a> {
    pub inbox: &'a Inbox,
    pub watches: &'a Watches,
    pub display: &'a Display<'a>,
    pub changes: &'a ChangesSection,
    pub store: &'a Path,
    pub monitor: &'a Monitor,
}

pub fn run_local(route: Route, ctx: &LocalRoute, refresh: impl Fn()) -> io::Result<()> {
    match route {
        Route::ListInbox => inbox::list(ctx.inbox)?,
        Route::PeekInbox(id) => inbox::peek(ctx.inbox, id, ctx.display)?,
        Route::DropInbox(id) => {
            inbox::drop_report(ctx.inbox, id)?;
            refresh();
        }
        Route::InboxUsage => inbox::usage(),
        Route::Watch(command) => {
            watch::run(command, ctx.watches);
            refresh();
        }
        Route::Diff(DiffArgs::Target(target)) => diff::run(
            &DiffContext {
                changes: ctx.changes,
                store: ctx.store,
            },
            target,
        ),
        Route::Diff(DiffArgs::Usage(usage)) => println!("{usage}"),
        Route::Diff(DiffArgs::UnclosedQuote) => println!("{UNCLOSED_DIFF_QUOTE}"),
        Route::Workers(id) => workers::run(ctx.monitor, id.as_deref(), ctx.display),
        Route::Help => print!("{}", help_text(screen_columns())),
        Route::Send(_) | Route::ReadInbox(_) | Route::Ignore => {}
    }
    Ok(())
}
