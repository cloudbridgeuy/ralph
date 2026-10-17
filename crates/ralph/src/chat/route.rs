use crate::chat::diff;
use crate::chat::inbox::{self, Display};
use crate::chat::watch;
use crate::config::ChangesSection;
use crate::watches::Watches;
use crate::worker_mcp::Inbox;
use ralph_core::chat_status::{Route, PRIVATE_USAGE};
use ralph_core::diff_command::{DiffArgs, UNCLOSED_DIFF_QUOTE};
use std::io;

pub struct LocalRoute<'a> {
    pub inbox: &'a Inbox,
    pub watches: &'a Watches,
    pub display: &'a Display<'a>,
    pub changes: &'a ChangesSection,
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
        Route::Diff(DiffArgs::Words(extra)) => diff::run(ctx.changes, &extra),
        Route::Diff(DiffArgs::UnclosedQuote) => println!("{UNCLOSED_DIFF_QUOTE}"),
        Route::PrivateUsage => println!("{PRIVATE_USAGE}"),
        Route::Send(_) | Route::Private(_) | Route::ReadInbox(_) | Route::Ignore => {}
    }
    Ok(())
}
