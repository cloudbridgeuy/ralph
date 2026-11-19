use crate::chat::client_monitor::Monitor;
use crate::chat::config;
use crate::chat::diff::{self, DiffContext};
use crate::chat::direct;
use crate::chat::inbox::{self, Display};
use crate::chat::nudge;
use crate::chat::prompt::Columns;
use crate::chat::rerender;
use crate::chat::schedule;
use crate::chat::thread::Memory;
use crate::chat::view;
use crate::chat::workers;
use crate::config::ConfigSnapshot;
use ralph_core::chat_help::help_with_store;
use ralph_core::chat_memory::{history_tool_of, CallSource, MemoryCall};
use ralph_core::chat_status::Route;
use ralph_core::diff_command::{DiffArgs, UNCLOSED_DIFF_QUOTE};
use ralph_core::wire::price::PriceOverride;
use ralph_core::worker::TicketKeys;
use ralph_server::chat::memory::Answer;
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
        Route::Help => write!(out, "{}", help_with_store((ctx.columns)(), ctx.store))?,
        Route::Config => config::print(ctx.config),
        Route::View(args) => view::run(&args, ctx.display),
        Route::Rerender(n) => rerender::run(&n, ctx.store, ctx.display),
        Route::History(call) => {
            let text = history_call(ctx.store, &call, CallSource::Slash)
                .map_or_else(|e| e, |answer| answer.text);
            writeln!(out, "{text}")?
        }
        Route::HistoryRefused { tool, message } => {
            history_log::record_refusal(ctx.store, tool, &message, CallSource::Slash);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::thread::thread_dir;
    use ralph_core::chat_memory::{history_path, parse_tool_call};
    use ralph_core::chat_thread::ThreadName;
    use serde_json::{json, Value};

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
