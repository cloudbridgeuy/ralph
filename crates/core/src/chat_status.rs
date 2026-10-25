use crate::chat_help::Command;
use crate::chat_hold::HoldFilter;
use crate::chat_thread::ThreadCell;
use crate::diff_command::{parse_diff_args, DiffArgs};
use crate::inbox::{Position, Selection};
use crate::status_queue::QueueCell;
use crate::watch_command::{format_left, parse_watch_args, time_left, WatchCommand};
use crate::watch_store::SavedWatch;
use crate::worker::{
    opencode_session_id, same_store, tagged_sessions, HarnessId, Report, ReportStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const DEFAULT_STATE: &str = "running";
const BUSY_STATE: &str = "busy";
const IDLE_STATE: &str = "idle";
const PERMISSION_STATE: &str = "permission";
pub const INBOX_USAGE: &str = "Usage: /inbox [next | N | list | peek N | drop N]";
const SEPARATOR: &str = " · ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerState {
    pub id: String,
    pub state: String,
}

impl WorkerState {
    pub fn is_running(&self) -> bool {
        [DEFAULT_STATE, BUSY_STATE, PERMISSION_STATE].contains(&self.state.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchState {
    pub id: String,
    pub left: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningWorker {
    pub id: String,
    pub session: String,
    #[serde(default, alias = "cursor")]
    pub kind: HarnessId,
}

impl RunningWorker {
    pub fn headless(id: &str, kind: HarnessId) -> Self {
        Self {
            id: id.to_string(),
            session: String::new(),
            kind,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub unread: usize,
    pub workers: Vec<WorkerState>,
    pub watches: Vec<WatchState>,
    pub thread: Option<ThreadCell>,
    pub napping: bool,
    pub queue: Option<QueueCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Send(String),
    ReadInbox(Selection),
    ListInbox,
    PeekInbox(Position),
    DropInbox(Position),
    InboxUsage,
    Watch(WatchCommand),
    Diff(DiffArgs),
    Workers(Option<String>),
    Hold(HoldFilter),
    Help,
    Ignore,
}

impl Route {
    pub fn runs_mid_turn(&self) -> bool {
        match self {
            Route::Workers(_)
            | Route::PeekInbox(_)
            | Route::ListInbox
            | Route::Help
            | Route::InboxUsage
            | Route::Watch(WatchCommand::List | WatchCommand::Usage)
            | Route::Diff(DiffArgs::Usage(_) | DiffArgs::UnclosedQuote) => true,
            Route::Send(_)
            | Route::ReadInbox(_)
            | Route::DropInbox(_)
            | Route::Hold(_)
            | Route::Watch(WatchCommand::Start(_) | WatchCommand::Stop(_))
            | Route::Diff(DiffArgs::Target(_))
            | Route::Ignore => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EmptySubmit {
    #[default]
    Inbox,
    InboxFirst(String),
    Phrase(String),
}

pub fn empty_route(unread: usize, empty: &EmptySubmit) -> Route {
    match empty {
        EmptySubmit::Phrase(phrase) => Route::Send(phrase.clone()),
        _ if unread > 0 => Route::ReadInbox(Selection::All),
        EmptySubmit::InboxFirst(phrase) => Route::Send(phrase.clone()),
        EmptySubmit::Inbox => Route::Ignore,
    }
}

pub fn route_input(line: &str, unread: usize, empty: &EmptySubmit) -> Route {
    let line = line.trim();
    for command in Command::ALL {
        if let Some(args) = command.args(line) {
            return match command {
                Command::Help => Route::Help,
                Command::Inbox => inbox_route(args),
                Command::Watch => Route::Watch(parse_watch_args(args)),
                Command::Diff => Route::Diff(parse_diff_args(args)),
                Command::Workers => workers_route(args, line),
                Command::Hold => hold_route(args, line),
            };
        }
    }
    match line {
        "" => empty_route(unread, empty),
        message => Route::Send(message.to_string()),
    }
}

fn workers_route(args: &str, line: &str) -> Route {
    let mut tokens = args.split_whitespace();
    match (tokens.next(), tokens.next()) {
        (None, _) => Route::Workers(None),
        (Some(id), None) => Route::Workers(Some(id.to_string())),
        _ => Route::Send(line.to_string()),
    }
}

fn hold_route(args: &str, line: &str) -> Route {
    HoldFilter::parse(args).map_or_else(|| Route::Send(line.to_string()), Route::Hold)
}

fn inbox_route(args: &str) -> Route {
    let mut tokens = args.split_whitespace();
    match (tokens.next(), tokens.next(), tokens.next()) {
        (None, ..) => Route::ReadInbox(Selection::All),
        (Some("next"), None, _) => Route::ReadInbox(Selection::Next),
        (Some("list"), None, _) => Route::ListInbox,
        (Some("peek"), Some(n), None) => n.parse().map_or(Route::InboxUsage, Route::PeekInbox),
        (Some("drop"), Some(n), None) => n.parse().map_or(Route::InboxUsage, Route::DropInbox),
        (Some(n), None, _) => n
            .parse()
            .map_or(Route::InboxUsage, |p| Route::ReadInbox(Selection::At(p))),
        _ => Route::InboxUsage,
    }
}

pub fn watch_states(saved: &[SavedWatch], now: chrono::DateTime<chrono::Utc>) -> Vec<WatchState> {
    saved
        .iter()
        .map(|watch| WatchState {
            id: watch.id.clone(),
            left: time_left(watch, now).map(format_left),
        })
        .collect()
}

pub fn store_workers(tmux_sessions: &str, store: &str) -> Vec<RunningWorker> {
    let mut workers: Vec<RunningWorker> = tagged_sessions(tmux_sessions)
        .into_iter()
        .filter(|session| same_store(&session.store, store) && !session.id.is_empty())
        .map(|session| RunningWorker {
            id: session.id,
            session: session.name,
            kind: HarnessId::Claude,
        })
        .collect();
    workers.sort_by(|a, b| a.id.cmp(&b.id));
    workers
}

pub fn orphan_hint(
    ready_ids: &[String],
    running: &[RunningWorker],
    acp_alive: &[String],
    resumable: &[String],
) -> Option<String> {
    let mut orphans: Vec<&str> = ready_ids
        .iter()
        .filter(|id| !ready_is_covered(id, running, acp_alive, resumable))
        .map(String::as_str)
        .collect();
    orphans.sort_unstable();
    (!orphans.is_empty()).then(|| {
        format!(
            "Leftover workers with no tmux session: {}. Ask the chat to close_worker them; nothing was removed.",
            orphans.join(", ")
        )
    })
}

/// A ready id is covered by a discovered running worker, a Cursor worker whose
/// `agent acp` process group is still the recorded one, or a Cursor worker that
/// still has a session record to resume.
fn ready_is_covered(
    id: &str,
    running: &[RunningWorker],
    acp_alive: &[String],
    resumable: &[String],
) -> bool {
    running.iter().any(|worker| worker.id == id)
        || acp_alive.iter().any(|alive| alive == id)
        || resumable.iter().any(|session| session == id)
}

pub fn latest_states(reports: &[Report]) -> HashMap<String, String> {
    reports
        .iter()
        .map(|r| (r.worker.clone(), r.status.clone()))
        .collect()
}

pub fn worker_states(
    workers: &[RunningWorker],
    seen: &HashMap<String, String>,
    sent: &HashSet<String>,
    active: Option<&HashSet<String>>,
    blocked: &HashSet<String>,
) -> Vec<WorkerState> {
    workers
        .iter()
        .map(|worker| WorkerState {
            id: worker.id.clone(),
            state: worker_state(
                worker,
                blocked.contains(&worker.id),
                seen.get(&worker.id)
                    .filter(|status| !superseded(status, sent.contains(&worker.id))),
                active,
            ),
        })
        .collect()
}

fn superseded(status: &str, sent: bool) -> bool {
    sent && ReportStatus::ALL
        .iter()
        .any(|reported| reported.as_str() == status)
}

fn worker_state(
    worker: &RunningWorker,
    permission_pending: bool,
    reported: Option<&String>,
    active: Option<&HashSet<String>>,
) -> String {
    let session_active = active
        .filter(|_| worker.kind == HarnessId::Opencode)
        .map(|active| active.contains(&opencode_session_id(&worker.id)));
    match (session_active, reported) {
        _ if permission_pending => PERMISSION_STATE,
        (Some(true), _) => BUSY_STATE,
        (_, Some(state)) => state,
        (Some(false), None) => IDLE_STATE,
        (None, None) => DEFAULT_STATE,
    }
    .to_string()
}

fn fit(candidates: Vec<String>, width: usize) -> String {
    let chosen = candidates
        .iter()
        .find(|line| line.width() <= width)
        .or(candidates.last());
    chosen
        .map(|line| truncate_to_width(line, width))
        .unwrap_or_default()
}

fn truncate_to_width(line: &str, width: usize) -> String {
    let mut used = 0;
    line.chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .collect()
}

pub fn footer(unread: usize, width: usize) -> String {
    let base = |inbox: Option<String>, full: bool| {
        let mut items = vec!["Enter send".to_string()];
        if full {
            items.push("Ctrl+J newline".to_string());
        }
        items.extend(inbox);
        if full {
            items.push("Ctrl+G editor".to_string());
        }
        items.push("Ctrl+D quit".to_string());
        items.join(SEPARATOR)
    };
    let reports = if unread == 1 { "report" } else { "reports" };
    let hint = |text: String| (unread > 0).then_some(text);
    let inbox = Command::Inbox.name();
    let candidates = vec![
        base(
            hint(format!(
                "{inbox} or Enter on empty prompt reads {unread} {reports}"
            )),
            true,
        ),
        base(hint(format!("{inbox} read {unread} {reports}")), true),
        base(hint(format!("{inbox} {unread}")), false),
        "Enter send".to_string(),
    ];
    format!("{}\n", fit(candidates, width))
}

#[cfg(test)]
mod tests;
