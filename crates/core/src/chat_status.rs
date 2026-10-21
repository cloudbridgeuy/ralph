use crate::diff_command::{parse_diff_args, DiffArgs};
use crate::inbox::{Position, Selection};
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
const INBOX_COMMAND: &str = "/inbox";
pub const INBOX_USAGE: &str = "Usage: /inbox [next | N | list | peek N | drop N]";
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
    pub fn cursor(id: &str) -> Self {
        Self {
            id: id.to_string(),
            session: String::new(),
            kind: HarnessId::Cursor,
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
        "" => empty_route(unread, empty),
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
) -> Option<String> {
    let mut orphans: Vec<&str> = ready_ids
        .iter()
        .filter(|id| !ready_is_covered(id, running, acp_alive))
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

/// A ready id is covered by a discovered running worker, or by a Cursor worker
/// whose `agent acp` process group is still the recorded one.
fn ready_is_covered(id: &str, running: &[RunningWorker], acp_alive: &[String]) -> bool {
    running.iter().any(|worker| worker.id == id) || acp_alive.iter().any(|alive| alive == id)
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
    use crate::changes::{ScopeQuery, TurnWindow};
    use crate::diff_command::{DiffArgs, DiffTarget, WINDOW_USAGE};

    #[test]
    fn last_turn_cache_picks_last_turn_and_ignores_the_rest() {
        let jsonl = concat!(
            r#"{"kind":"turn","input_tokens":1,"cache_creation_input_tokens":2,"cache_read_input_tokens":3,"cost_usd":0.5}"#,
            "\n",
            r#"{"kind":"turn","input_tokens":10,"cache_creation_input_tokens":20,"cache_read_input_tokens":30,"cost_usd":null,"system_bytes":5,"prompt_bytes":9,"agents_bytes":2}"#,
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
            route_input("/watch list", 0, &EmptySubmit::Inbox),
            Route::Watch(WatchCommand::List)
        );
        assert_eq!(
            route_input("/watch", 3, &EmptySubmit::Inbox),
            Route::Watch(WatchCommand::Usage)
        );
        assert_eq!(
            route_input(" /watch stop w1 ", 0, &EmptySubmit::Inbox),
            Route::Watch(WatchCommand::Stop("watch:w1".into()))
        );
        assert!(matches!(
            route_input(
                "/watch start --persistent sleep 5 -- done",
                0,
                &EmptySubmit::Inbox
            ),
            Route::Watch(WatchCommand::Start(_))
        ));
        assert_eq!(
            route_input("/watchfoo", 0, &EmptySubmit::Inbox),
            Route::Send("/watchfoo".into())
        );
    }

    #[test]
    fn route_input_parses_diff_commands() {
        let last = |count, extra: &[&str]| {
            Route::Diff(DiffArgs::Target(DiffTarget::Range {
                scope: ScopeQuery::Chat(TurnWindow::Last(count)),
                extra: extra.iter().map(|word| word.to_string()).collect(),
            }))
        };
        assert_eq!(
            route_input("/diff -U1 -- a", 0, &EmptySubmit::Inbox),
            last(1, &["-U1", "--", "a"])
        );
        assert_eq!(
            route_input(" /diff -- \"a b\" ", 3, &EmptySubmit::Inbox),
            last(1, &["--", "a b"])
        );
        assert_eq!(route_input("/diff", 0, &EmptySubmit::Inbox), last(1, &[]));
        assert_eq!(
            route_input("/diff last 2", 0, &EmptySubmit::Inbox),
            last(2, &[])
        );
        assert_eq!(
            route_input("/diff now -U1", 0, &EmptySubmit::Inbox),
            Route::Diff(DiffArgs::Target(DiffTarget::Now(vec!["-U1".into()])))
        );
        assert_eq!(
            route_input("/diff last 0", 0, &EmptySubmit::Inbox),
            Route::Diff(DiffArgs::Usage(WINDOW_USAGE))
        );
        assert_eq!(
            route_input("/diff 'a", 0, &EmptySubmit::Inbox),
            Route::Diff(DiffArgs::UnclosedQuote)
        );
        assert_eq!(
            route_input("/difffoo", 0, &EmptySubmit::Inbox),
            Route::Send("/difffoo".into())
        );
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
    fn empty_route_decides_from_mode_and_unread() {
        let phrase = || "continue".to_string();
        let send = || Route::Send(phrase());
        let read = || Route::ReadInbox(Selection::All);
        assert_eq!(empty_route(0, &EmptySubmit::Inbox), Route::Ignore);
        assert_eq!(empty_route(2, &EmptySubmit::Inbox), read());
        assert_eq!(empty_route(0, &EmptySubmit::InboxFirst(phrase())), send());
        assert_eq!(empty_route(2, &EmptySubmit::InboxFirst(phrase())), read());
        assert_eq!(empty_route(0, &EmptySubmit::Phrase(phrase())), send());
        assert_eq!(empty_route(2, &EmptySubmit::Phrase(phrase())), send());
    }

    #[test]
    fn route_input_sends_phrase_only_on_empty_line() {
        let phrase = EmptySubmit::Phrase("go".into());
        assert_eq!(route_input("  ", 3, &phrase), Route::Send("go".into()));
        assert_eq!(route_input("hi", 3, &phrase), Route::Send("hi".into()));
        assert_eq!(
            route_input("/inbox", 3, &phrase),
            Route::ReadInbox(Selection::All)
        );
    }

    #[test]
    fn route_input_decides_from_line_and_unread() {
        assert_eq!(
            route_input(" /inbox ", 0, &EmptySubmit::Inbox),
            Route::ReadInbox(Selection::All)
        );
        assert_eq!(
            route_input("  ", 3, &EmptySubmit::Inbox),
            Route::ReadInbox(Selection::All)
        );
        assert_eq!(
            route_input("/inbox next", 0, &EmptySubmit::Inbox),
            Route::ReadInbox(Selection::Next)
        );
        assert_eq!(
            route_input("/inbox 2", 0, &EmptySubmit::Inbox),
            Route::ReadInbox(Selection::At("2".parse().unwrap()))
        );
        assert_eq!(route_input("", 0, &EmptySubmit::Inbox), Route::Ignore);
        assert_eq!(
            route_input(" hi ", 3, &EmptySubmit::Inbox),
            Route::Send("hi".into())
        );
        assert_eq!(
            route_input("/inbox list", 0, &EmptySubmit::Inbox),
            Route::ListInbox
        );
        assert_eq!(
            route_input("/inbox peek 3", 0, &EmptySubmit::Inbox),
            Route::PeekInbox("3".parse().unwrap())
        );
        assert_eq!(
            route_input("/inbox drop 3", 0, &EmptySubmit::Inbox),
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
            assert_eq!(
                route_input(usage, 3, &EmptySubmit::Inbox),
                Route::InboxUsage
            );
        }
        assert_eq!(
            route_input("/inboxfoo", 0, &EmptySubmit::Inbox),
            Route::Send("/inboxfoo".into())
        );
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
        let states = worker_states(
            &workers,
            &latest_states(&reports),
            &HashSet::new(),
            None,
            &HashSet::new(),
        );
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
                kind: HarnessId::Claude
            }]
        );
    }

    #[test]
    fn running_worker_loads_legacy_cursor_records() {
        for (cursor, kind) in [(true, HarnessId::Cursor), (false, HarnessId::Claude)] {
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
    fn running_worker_loads_explicit_kinds_and_defaults_to_claude() {
        for (name, kind) in [
            ("tmux", HarnessId::Claude),
            ("claude", HarnessId::Claude),
            ("cursor", HarnessId::Cursor),
            ("opencode", HarnessId::Opencode),
        ] {
            let record = serde_json::json!({
                "id": "A", "session": "", "opencode": false, "kind": name
            });
            let worker: RunningWorker = serde_json::from_value(record).unwrap();
            assert_eq!(worker.kind, kind);
            assert_eq!(serde_json::to_value(worker).unwrap()["kind"], kind.as_str());
        }
        let worker: RunningWorker =
            serde_json::from_str(r#"{"id":"A","session":"s","opencode":false}"#).unwrap();
        assert_eq!(worker.kind, HarnessId::Claude);
    }

    #[test]
    fn orphan_hint_lists_ready_ids_without_a_session() {
        let running = [RunningWorker {
            id: "A".into(),
            session: "ralph-p-A".into(),
            kind: HarnessId::Claude,
        }];
        let ready = ["C".to_string(), "A".to_string(), "B".to_string()];
        let hint = orphan_hint(&ready, &running, &[]).unwrap();
        assert!(hint.contains(": B, C."), "{hint}");
        assert!(hint.contains("nothing was removed"));
        assert_eq!(orphan_hint(&ready[1..2], &running, &[]), None);
        assert_eq!(orphan_hint(&[], &running, &[]), None);
    }

    #[test]
    fn orphan_hint_skips_an_acp_alive_cursor_worker() {
        let running = [
            RunningWorker {
                id: "claude-up".into(),
                session: "ralph-p-claude-up".into(),
                kind: HarnessId::Claude,
            },
            RunningWorker::cursor("cursor-owned"),
        ];
        let ready = [
            "cursor-live".to_string(),
            "cursor-dead".to_string(),
            "cursor-owned".to_string(),
            "claude-down".to_string(),
            "claude-up".to_string(),
        ];
        let alive = ["cursor-live".to_string()];
        let hint = orphan_hint(&ready, &running, &alive).unwrap();
        assert!(hint.contains("cursor-dead"), "{hint}");
        assert!(hint.contains("claude-down"), "{hint}");
        assert!(!hint.contains("cursor-live"), "{hint}");
        assert!(!hint.contains("cursor-owned"), "{hint}");
        assert!(!hint.contains("claude-up"), "{hint}");
        assert_eq!(orphan_hint(&["cursor-live".to_string()], &[], &alive), None);
    }

    #[test]
    fn opencode_workers_are_busy_while_their_session_is_active() {
        let worker = |id: &str, kind| RunningWorker {
            id: id.into(),
            session: format!("ralph-p-{id}"),
            kind,
        };
        let workers = [
            worker("busy", HarnessId::Opencode),
            worker("idle", HarnessId::Opencode),
            worker("done", HarnessId::Opencode),
            worker("redo", HarnessId::Opencode),
            worker("claude", HarnessId::Claude),
        ];
        let seen = HashMap::from([
            ("done".to_string(), "done".to_string()),
            ("redo".to_string(), "done".to_string()),
        ]);
        let active = HashSet::from(["ses_ralphbusy".to_string(), "ses_ralphredo".to_string()]);
        let states = |active| -> Vec<String> {
            worker_states(&workers, &seen, &HashSet::new(), active, &HashSet::new())
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
        let worker = |id: &str, kind| RunningWorker {
            id: id.into(),
            session: format!("ralph-p-{id}"),
            kind,
        };
        let workers = [
            worker("redone", HarnessId::Claude),
            worker("quiet", HarnessId::Claude),
            worker("checked", HarnessId::Claude),
            worker("ask", HarnessId::Claude),
            worker("busy", HarnessId::Opencode),
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
        let states: Vec<String> =
            worker_states(&workers, &seen, &sent, Some(&active), &HashSet::new())
                .into_iter()
                .map(|worker| worker.state)
                .collect();
        assert_eq!(states, vec!["running", "done", "check", "running", "busy"]);
    }

    #[test]
    fn a_pending_permission_overrides_a_report_and_counts_as_running() {
        let workers = [RunningWorker {
            id: "w".into(),
            session: "ralph-p-w".into(),
            kind: HarnessId::Cursor,
        }];
        let seen = HashMap::from([("w".to_string(), "question".to_string())]);
        let blocked = HashSet::from(["w".to_string()]);
        let states = worker_states(&workers, &seen, &HashSet::new(), None, &blocked);
        assert_eq!(states[0].state, "permission");
        assert!(states[0].is_running());
    }

    #[test]
    fn memory_size_is_chars_over_four() {
        let lines = vec!["abcdefgh".to_string(), "ééééééé".to_string()];
        assert_eq!(memory_size(2, &lines), Memory { msgs: 2, tokens: 3 });
    }
}
