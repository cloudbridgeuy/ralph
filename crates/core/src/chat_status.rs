use crate::diff_command::{parse_diff_args, DiffArgs};
use crate::inbox::{Position, Selection};
use crate::watch_command::{format_left, parse_watch_args, time_left, WatchCommand};
use crate::watch_store::SavedWatch;
use crate::worker::{
    opencode_session_id, same_store, tagged_sessions, Report, ReportStatus, WorkerKind,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const DEFAULT_STATE: &str = "running";
const BUSY_STATE: &str = "busy";
const IDLE_STATE: &str = "idle";
const INBOX_COMMAND: &str = "/inbox";
pub const INBOX_USAGE: &str = "Usage: /inbox [next | N | list | peek N | drop N]";
pub const PRIVATE_USAGE: &str = "Usage: /private <message>";
const PRIVATE_COMMAND: &str = "/private";
const WATCH_COMMAND: &str = "/watch";
const DIFF_COMMAND: &str = "/diff";
const SEPARATOR: &str = " · ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerState {
    pub id: String,
    pub state: String,
}

impl WorkerState {
    pub fn is_running(&self) -> bool {
        self.state == DEFAULT_STATE || self.state == BUSY_STATE
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
    pub opencode: bool,
    #[serde(default, alias = "cursor")]
    pub kind: WorkerKind,
}

impl RunningWorker {
    pub fn cursor(id: &str) -> Self {
        Self {
            id: id.to_string(),
            session: String::new(),
            opencode: false,
            kind: WorkerKind::Cursor,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    pub msgs: usize,
    pub tokens: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TurnCache {
    pub input: u64,
    pub read: u64,
    pub write: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    pub unread: usize,
    pub workers: Vec<WorkerState>,
    pub watches: Vec<WatchState>,
    pub memory: Memory,
    pub turn: Option<TurnCache>,
    pub napping: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UsageLine {
    kind: String,
    input_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    cost_usd: Option<f64>,
}

pub fn last_turn_cache(usage_jsonl: &str) -> Option<TurnCache> {
    usage_jsonl
        .lines()
        .filter_map(|line| serde_json::from_str::<UsageLine>(line).ok())
        .rfind(|usage| usage.kind == "turn")
        .map(|usage| TurnCache {
            input: usage.input_tokens,
            read: usage.cache_read_input_tokens,
            write: usage.cache_creation_input_tokens,
            cost_usd: usage.cost_usd,
        })
}

impl TurnCache {
    pub(crate) fn token_share_percent(&self) -> u64 {
        let total = self.input + self.read + self.write;
        (self.read * 100).checked_div(total).unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Send(String),
    Private(String),
    PrivateUsage,
    ReadInbox(Selection),
    ListInbox,
    PeekInbox(Position),
    DropInbox(Position),
    InboxUsage,
    Watch(WatchCommand),
    Diff(DiffArgs),
    Ignore,
}

pub fn memory_size(msgs: usize, view_lines: &[String]) -> Memory {
    let chars: usize = view_lines.iter().map(|line| line.chars().count()).sum();
    Memory {
        msgs,
        tokens: chars / 4,
    }
}

pub fn route_input(line: &str, unread: usize) -> Route {
    let line = line.trim();
    if let Some(args) = line
        .strip_prefix(WATCH_COMMAND)
        .filter(|args| args.is_empty() || args.starts_with(char::is_whitespace))
    {
        return Route::Watch(parse_watch_args(args));
    }
    if let Some(args) = line
        .strip_prefix(DIFF_COMMAND)
        .filter(|args| args.is_empty() || args.starts_with(char::is_whitespace))
    {
        return Route::Diff(parse_diff_args(args));
    }
    if let Some(text) = line
        .strip_prefix(PRIVATE_COMMAND)
        .filter(|text| text.is_empty() || text.starts_with(char::is_whitespace))
    {
        return match text.trim() {
            "" => Route::PrivateUsage,
            message => Route::Private(message.to_string()),
        };
    }
    let mut tokens = line.split_whitespace();
    if tokens.next() == Some(INBOX_COMMAND) {
        return match (tokens.next(), tokens.next(), tokens.next()) {
            (None, ..) => Route::ReadInbox(Selection::All),
            (Some("next"), None, _) => Route::ReadInbox(Selection::Next),
            (Some("list"), None, _) => Route::ListInbox,
            (Some("peek"), Some(n), None) => n.parse().map_or(Route::InboxUsage, Route::PeekInbox),
            (Some("drop"), Some(n), None) => n.parse().map_or(Route::InboxUsage, Route::DropInbox),
            (Some(n), None, _) => n
                .parse()
                .map_or(Route::InboxUsage, |p| Route::ReadInbox(Selection::At(p))),
            _ => Route::InboxUsage,
        };
    }
    match line {
        "" if unread > 0 => Route::ReadInbox(Selection::All),
        "" => Route::Ignore,
        message => Route::Send(message.to_string()),
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
            opencode: session.opencode,
            kind: WorkerKind::Tmux,
        })
        .collect();
    workers.sort_by(|a, b| a.id.cmp(&b.id));
    workers
}

pub fn orphan_hint(ready_ids: &[String], running: &[RunningWorker]) -> Option<String> {
    let mut orphans: Vec<&str> = ready_ids
        .iter()
        .filter(|id| !running.iter().any(|worker| &worker.id == *id))
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
) -> Vec<WorkerState> {
    workers
        .iter()
        .map(|worker| WorkerState {
            id: worker.id.clone(),
            state: worker_state(
                worker,
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
    reported: Option<&String>,
    active: Option<&HashSet<String>>,
) -> String {
    let session_active = active
        .filter(|_| worker.opencode)
        .map(|active| active.contains(&opencode_session_id(&worker.id)));
    match (session_active, reported) {
        (Some(true), _) => BUSY_STATE,
        (_, Some(state)) => state,
        (Some(false), None) => IDLE_STATE,
        (None, None) => DEFAULT_STATE,
    }
    .to_string()
}

pub(crate) fn tokens_label(tokens: u64) -> String {
    if tokens < 1000 {
        tokens.to_string()
    } else {
        format!("{}.{}k", tokens / 1000, tokens % 1000 / 100)
    }
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
    let candidates = vec![
        base(
            hint(format!(
                "{INBOX_COMMAND} or Enter on empty prompt reads {unread} {reports}"
            )),
            true,
        ),
        base(
            hint(format!("{INBOX_COMMAND} read {unread} {reports}")),
            true,
        ),
        base(hint(format!("{INBOX_COMMAND} {unread}")), false),
        "Enter send".to_string(),
    ];
    format!("{}\n", fit(candidates, width))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_command::DiffArgs;

    #[test]
    fn last_turn_cache_picks_last_turn_and_ignores_the_rest() {
        let jsonl = concat!(
            r#"{"kind":"turn","input_tokens":1,"cache_creation_input_tokens":2,"cache_read_input_tokens":3,"cost_usd":0.5}"#,
            "\n",
            r#"{"kind":"turn","input_tokens":10,"cache_creation_input_tokens":20,"cache_read_input_tokens":30,"cost_usd":null}"#,
            "\n",
            r#"{"kind":"nap","input_tokens":99}"#,
            "\nnot json\n"
        );
        assert_eq!(
            last_turn_cache(jsonl),
            Some(TurnCache {
                input: 10,
                read: 30,
                write: 20,
                cost_usd: None
            })
        );
        assert_eq!(last_turn_cache(""), None);
        assert_eq!(last_turn_cache("garbage\n{\"kind\":\"nap\"}\n"), None);
    }

    #[test]
    pub(crate) fn token_share_percent_is_zero_without_tokens() {
        assert_eq!(TurnCache::default().token_share_percent(), 0);
    }

    #[test]
    fn fit_truncates_wide_chars_by_display_width() {
        let line = "📬 2 unread".to_string();
        for width in 0..=12 {
            let fitted = fit(vec![line.clone()], width);
            assert!(fitted.width() <= width, "width {width}: {fitted:?}");
        }
        assert_eq!(fit(vec![line], 1), "");
    }

    #[test]
    fn watch_states_report_the_time_left() {
        use crate::watch::{WatchKind, WatchSpec};
        let now = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let spec = |id: &str, command: &str| WatchSpec {
            id: id.into(),
            command: command.into(),
            follow_up: None,
            kind: WatchKind::Normal,
            timeout: None,
            limit: None,
        };
        let saved = [
            SavedWatch::new(&spec("w1", "sleep 90"), now),
            SavedWatch::new(&spec("w2", "make"), now),
        ];
        assert_eq!(
            watch_states(&saved, now + chrono::Duration::seconds(30)),
            vec![
                WatchState {
                    id: "w1".into(),
                    left: Some("1m".into())
                },
                WatchState {
                    id: "w2".into(),
                    left: None
                }
            ]
        );
    }

    #[test]
    fn watch_states_are_equal_one_second_apart_at_twelve_minutes() {
        use crate::watch::{WatchKind, WatchSpec};
        let now = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let spec = WatchSpec {
            id: "w1".into(),
            command: "sleep 3600".into(),
            follow_up: None,
            kind: WatchKind::Normal,
            timeout: None,
            limit: None,
        };
        let saved = [SavedWatch::new(&spec, now)];
        let at = |secs| watch_states(&saved, now + chrono::Duration::seconds(secs));
        assert_eq!(at(2870), at(2871));
        assert_eq!(at(2870)[0].left.as_deref(), Some("12m"));
    }

    #[test]
    fn route_input_parses_watch_commands() {
        assert_eq!(
            route_input("/watch list", 0),
            Route::Watch(WatchCommand::List)
        );
        assert_eq!(route_input("/watch", 3), Route::Watch(WatchCommand::Usage));
        assert_eq!(
            route_input(" /watch stop w1 ", 0),
            Route::Watch(WatchCommand::Stop("watch:w1".into()))
        );
        assert!(matches!(
            route_input("/watch start --persistent sleep 5 -- done", 0),
            Route::Watch(WatchCommand::Start(_))
        ));
        assert_eq!(route_input("/watchfoo", 0), Route::Send("/watchfoo".into()));
    }

    #[test]
    fn route_input_parses_diff_commands() {
        assert_eq!(
            route_input("/diff -U1 -- a", 0),
            Route::Diff(DiffArgs::Words(vec!["-U1".into(), "--".into(), "a".into()]))
        );
        assert_eq!(
            route_input(" /diff -- \"a b\" ", 3),
            Route::Diff(DiffArgs::Words(vec!["--".into(), "a b".into()]))
        );
        assert_eq!(
            route_input("/diff", 0),
            Route::Diff(DiffArgs::Words(vec![]))
        );
        assert_eq!(
            route_input("/diff 'a", 0),
            Route::Diff(DiffArgs::UnclosedQuote)
        );
        assert_eq!(route_input("/difffoo", 0), Route::Send("/difffoo".into()));
    }

    #[test]
    fn route_input_parses_private_turns() {
        assert_eq!(
            route_input(" /private  what is x? ", 2),
            Route::Private("what is x?".into())
        );
        assert_eq!(
            route_input("/private\nline one\nline two", 0),
            Route::Private("line one\nline two".into())
        );
        assert_eq!(
            route_input("/privatefoo", 0),
            Route::Send("/privatefoo".into())
        );
    }

    #[test]
    fn bare_private_shows_the_usage_hint_and_sends_nothing() {
        for line in ["/private", "/private ", "  /private \t ", "/private\n"] {
            assert_eq!(route_input(line, 3), Route::PrivateUsage);
        }
    }

    #[test]
    fn footer_shows_inbox_hint_only_with_mail() {
        assert!(!footer(0, 200).contains("/inbox"));
        assert_eq!(
            footer(2, 200),
            "Enter send · Ctrl+J newline · /inbox or Enter on empty prompt reads 2 reports · Ctrl+G editor · Ctrl+D quit\n"
        );
        assert!(footer(2, 90).contains("/inbox read 2 reports"));
        assert!((0..120).all(|w| footer(2, w).trim_end().width() <= w));
    }

    #[test]
    fn route_input_decides_from_line_and_unread() {
        assert_eq!(route_input(" /inbox ", 0), Route::ReadInbox(Selection::All));
        assert_eq!(route_input("  ", 3), Route::ReadInbox(Selection::All));
        assert_eq!(
            route_input("/inbox next", 0),
            Route::ReadInbox(Selection::Next)
        );
        assert_eq!(
            route_input("/inbox 2", 0),
            Route::ReadInbox(Selection::At("2".parse().unwrap()))
        );
        assert_eq!(route_input("", 0), Route::Ignore);
        assert_eq!(route_input(" hi ", 3), Route::Send("hi".into()));
        assert_eq!(route_input("/inbox list", 0), Route::ListInbox);
        assert_eq!(
            route_input("/inbox peek 3", 0),
            Route::PeekInbox("3".parse().unwrap())
        );
        assert_eq!(
            route_input("/inbox drop 3", 0),
            Route::DropInbox("3".parse().unwrap())
        );
        for usage in [
            "/inbox drop x",
            "/inbox drop",
            "/inbox drop 1 2",
            "/inbox peek x",
            "/inbox peek",
            "/inbox foo",
            "/inbox list x",
            "/inbox peek 1 2",
            "/inbox 0",
            "/inbox x",
            "/inbox 2 3",
            "/inbox next 1",
        ] {
            assert_eq!(route_input(usage, 3), Route::InboxUsage);
        }
        assert_eq!(route_input("/inboxfoo", 0), Route::Send("/inboxfoo".into()));
    }

    #[test]
    fn store_workers_keep_only_this_stores_sessions() {
        let listing =
            "ralph-p-A\t/s\tA\nralph-p-B\t/other\tB\nralph-p-C\t\tC\nralph-D\t/s\nmain\t/s\n";
        let ids = |store| -> Vec<String> {
            store_workers(listing, store)
                .into_iter()
                .map(|worker| worker.id)
                .collect()
        };
        assert_eq!(ids("/s"), vec!["A"]);
        assert_eq!(ids("/other"), vec!["B"]);
        assert!(store_workers("", "/s").is_empty());
    }

    #[test]
    fn workers_come_from_prefixed_sessions_with_last_known_state() {
        let listing =
            "main\t/s\nralph-p-GUZ-2\t/s\tGUZ-2\nralph-p-GUZ-1\t/s\tGUZ-1\nralph-\t/s\t\n";
        let workers = store_workers(listing, "/s");
        let ids: Vec<&str> = workers.iter().map(|worker| worker.id.as_str()).collect();
        assert_eq!(ids, vec!["GUZ-1", "GUZ-2"]);
        let reports = [
            Report {
                worker: "GUZ-2".into(),
                at: "a".into(),
                status: "question".into(),
                text: "x".into(),
            },
            Report {
                worker: "GUZ-2".into(),
                at: "b".into(),
                status: "done".into(),
                text: "y".into(),
            },
        ];
        let states = worker_states(&workers, &latest_states(&reports), &HashSet::new(), None);
        assert_eq!(states[0].state, "running");
        assert_eq!(states[1].state, "done");
    }

    #[test]
    fn store_workers_take_the_bare_id_from_the_id_tag() {
        let workers = store_workers("ralph-my-proj-GUZ-1\t/s\tGUZ-1\n", "/s");
        assert_eq!(
            workers,
            vec![RunningWorker {
                id: "GUZ-1".into(),
                session: "ralph-my-proj-GUZ-1".into(),
                opencode: false,
                kind: WorkerKind::Tmux
            }]
        );
    }

    #[test]
    fn running_worker_loads_legacy_cursor_records() {
        for (cursor, kind) in [(true, WorkerKind::Cursor), (false, WorkerKind::Tmux)] {
            let record = serde_json::json!({
                "id": "A", "session": "", "opencode": false, "cursor": cursor
            });
            let worker: RunningWorker = serde_json::from_value(record).unwrap();
            assert_eq!(worker.kind, kind);
            let saved = serde_json::to_value(&worker).unwrap();
            assert!(saved.get("cursor").is_none());
            assert_eq!(
                serde_json::from_value::<RunningWorker>(saved).unwrap(),
                worker
            );
        }
    }

    #[test]
    fn running_worker_loads_explicit_kinds_and_defaults_to_tmux() {
        for (name, kind) in [
            ("tmux", WorkerKind::Tmux),
            ("cursor", WorkerKind::Cursor),
            ("opencode", WorkerKind::Opencode),
        ] {
            let record = serde_json::json!({
                "id": "A", "session": "", "opencode": false, "kind": name
            });
            let worker: RunningWorker = serde_json::from_value(record).unwrap();
            assert_eq!(worker.kind, kind);
            assert_eq!(serde_json::to_value(worker).unwrap()["kind"], name);
        }
        let worker: RunningWorker =
            serde_json::from_str(r#"{"id":"A","session":"s","opencode":false}"#).unwrap();
        assert_eq!(worker.kind, WorkerKind::Tmux);
    }

    #[test]
    fn orphan_hint_lists_ready_ids_without_a_session() {
        let running = [RunningWorker {
            id: "A".into(),
            session: "ralph-p-A".into(),
            opencode: false,
            kind: WorkerKind::Tmux,
        }];
        let ready = ["C".to_string(), "A".to_string(), "B".to_string()];
        let hint = orphan_hint(&ready, &running).unwrap();
        assert!(hint.contains(": B, C."), "{hint}");
        assert!(hint.contains("nothing was removed"));
        assert_eq!(orphan_hint(&ready[1..2], &running), None);
        assert_eq!(orphan_hint(&[], &running), None);
    }

    #[test]
    fn opencode_workers_are_busy_while_their_session_is_active() {
        let worker = |id: &str, opencode| RunningWorker {
            id: id.into(),
            session: format!("ralph-p-{id}"),
            opencode,
            kind: WorkerKind::Tmux,
        };
        let workers = [
            worker("busy", true),
            worker("idle", true),
            worker("done", true),
            worker("redo", true),
            worker("claude", false),
        ];
        let seen = HashMap::from([
            ("done".to_string(), "done".to_string()),
            ("redo".to_string(), "done".to_string()),
        ]);
        let active = HashSet::from(["ses_ralphbusy".to_string(), "ses_ralphredo".to_string()]);
        let states = |active| -> Vec<String> {
            worker_states(&workers, &seen, &HashSet::new(), active)
                .into_iter()
                .map(|worker| worker.state)
                .collect()
        };
        assert_eq!(
            states(Some(&active)),
            vec!["busy", "idle", "done", "busy", "running"]
        );
        assert_eq!(
            states(None),
            vec!["running", "running", "done", "done", "running"]
        );
    }

    #[test]
    fn a_sent_message_drops_a_report_status_but_keeps_other_statuses() {
        let worker = |id: &str, opencode| RunningWorker {
            id: id.into(),
            session: format!("ralph-p-{id}"),
            opencode,
            kind: WorkerKind::Tmux,
        };
        let workers = [
            worker("redone", false),
            worker("quiet", false),
            worker("checked", false),
            worker("ask", false),
            worker("busy", true),
        ];
        let seen = HashMap::from([
            ("redone".to_string(), "done".to_string()),
            ("quiet".to_string(), "done".to_string()),
            ("checked".to_string(), "check".to_string()),
            ("ask".to_string(), "question".to_string()),
            ("busy".to_string(), "done".to_string()),
        ]);
        let sent: HashSet<String> = ["redone", "checked", "ask", "busy"]
            .map(String::from)
            .into();
        let active = HashSet::from(["ses_ralphbusy".to_string()]);
        let states: Vec<String> = worker_states(&workers, &seen, &sent, Some(&active))
            .into_iter()
            .map(|worker| worker.state)
            .collect();
        assert_eq!(states, vec!["running", "done", "check", "running", "busy"]);
    }

    #[test]
    fn memory_size_is_chars_over_four() {
        let lines = vec!["abcdefgh".to_string(), "ééééééé".to_string()];
        assert_eq!(memory_size(2, &lines), Memory { msgs: 2, tokens: 3 });
    }
}
