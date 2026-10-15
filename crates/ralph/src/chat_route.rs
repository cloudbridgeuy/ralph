use crate::chat_inbox::{self, Display};
use crate::chat_watch;
use crate::watches::Watches;
use crate::worker_mcp::Inbox;
use ralph_core::chat_status::Route;
use std::io;

pub struct LocalRoute<'a> {
    pub inbox: &'a Inbox,
    pub watches: &'a Watches,
    pub display: &'a Display<'a>,
}

pub fn run_local(route: Route, ctx: &LocalRoute, refresh: impl Fn()) -> io::Result<()> {
    match route {
        Route::ListInbox => chat_inbox::list(ctx.inbox)?,
        Route::PeekInbox(id) => chat_inbox::peek(ctx.inbox, id, ctx.display)?,
        Route::DropInbox(id) => {
            chat_inbox::drop_report(ctx.inbox, id)?;
            refresh();
        }
        Route::InboxUsage => chat_inbox::usage(),
        Route::Watch(command) => {
            chat_watch::run(command, ctx.watches);
            refresh();
        }
        Route::Send(_) | Route::ReadInbox(_) | Route::Ignore => {}
    }
    Ok(())
}
