use crate::inbox::ReportId;
use crate::worker::{opencode_session_id, same_store, tagged_sessions, Report};
use serde::de::IgnoredAny;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const DEFAULT_STATE: &str = "running";
const BUSY_STATE: &str = "busy";
const IDLE_STATE: &str = "idle";
const INBOX_COMMAND: &str = "/inbox";
pub const INBOX_USAGE: &str = "Usage: /inbox [list | peek N | drop N]";
const SEPARATOR: &str = " · ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerState {
    pub id: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningWorker {
    pub id: String,
    pub session: String,
    pub opencode: bool,
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
    pub memory: Memory,
    pub turn: Option<TurnCache>,
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
    fn hit_percent(&self) -> u64 {
        let total = self.input + self.read + self.write;
        (self.read * 100).checked_div(total).unwrap_or(0)
    }

    fn cost_label(&self) -> String {
        self.cost_usd
            .map(|cost| format!(" ${cost:.2}"))
            .unwrap_or_default()
    }

    fn long_label(&self) -> String {
        format!(
            "cache {} read {} write {}% hit{}",
            tokens_label(self.read),
            tokens_label(self.write),
            self.hit_percent(),
            self.cost_label()
        )
    }

    fn compact_label(&self) -> String {
        format!("c{}%{}", self.hit_percent(), self.cost_label())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Send(String),
    ReadInbox,
    ListInbox,
    PeekInbox(ReportId),
    DropInbox(ReportId),
    InboxUsage,
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
    let mut tokens = line.split_whitespace();
    if tokens.next() == Some(INBOX_COMMAND) {
        return match (tokens.next(), tokens.next(), tokens.next()) {
            (None, ..) => Route::ReadInbox,
            (Some("list"), None, _) => Route::ListInbox,
            (Some("peek"), Some(n), None) => n.parse().map_or(Route::InboxUsage, Route::PeekInbox),
            (Some("drop"), Some(n), None) => n.parse().map_or(Route::InboxUsage, Route::DropInbox),
            _ => Route::InboxUsage,
        };
    }
    match line {
        "" if unread > 0 => Route::ReadInbox,
        "" => Route::Ignore,
        message => Route::Send(message.to_string()),
    }
}

pub fn store_workers(tmux_sessions: &str, store: &str) -> Vec<RunningWorker> {
    let mut workers: Vec<RunningWorker> = tagged_sessions(tmux_sessions)
        .into_iter()
        .filter(|session| same_store(&session.store, store) && !session.id.is_empty())
        .map(|session| RunningWorker {
            id: session.id,
            session: session.name,
            opencode: session.opencode,
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

#[derive(Deserialize)]
struct ActiveSessions {
    data: HashMap<String, IgnoredAny>,
}

/// Session ids in `opencode api session.active` output; sessions absent from it are idle.
pub fn active_sessions(json: &str) -> Option<HashSet<String>> {
    serde_json::from_str::<ActiveSessions>(json)
        .ok()
        .map(|active| active.data.into_keys().collect())
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
    active: Option<&HashSet<String>>,
) -> Vec<WorkerState> {
    workers
        .iter()
        .map(|worker| WorkerState {
            id: worker.id.clone(),
            state: worker_state(worker, seen.get(&worker.id), active),
        })
        .collect()
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

fn tokens_label(tokens: u64) -> String {
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

pub fn status_line(status: &Status, width: usize) -> String {
    let workers = &status.workers;
    let count = workers.len();
    let named = |with_state: bool| {
        let names: Vec<String> = workers
            .iter()
            .map(|w| {
                if with_state {
                    format!("{} {}", w.id, w.state)
                } else {
                    w.id.clone()
                }
            })
            .collect();
        format!("workers {}", names.join(", "))
    };
    let Memory { msgs, tokens } = status.memory;
    let tok = tokens_label(u64::try_from(tokens).unwrap_or(u64::MAX));
    let unread = status.unread;
    let long_cache = status.turn.map(|turn| turn.long_label());
    let compact_cache = status.turn.map(|turn| turn.compact_label());
    let build = |mail: String, workers: Option<String>, mem: String, cache: Option<String>| {
        [Some(mail), workers, Some(mem), cache]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(SEPARATOR)
    };
    let long_mail = || format!("📬 {unread} unread");
    let long_mem = || format!("mem {msgs} msgs ~{tok} tok");
    let compact_mem = || format!("{msgs}m ~{tok}");
    let candidates = vec![
        build(
            long_mail(),
            (count > 0).then(|| named(true)),
            long_mem(),
            long_cache.clone(),
        ),
        build(
            long_mail(),
            (count > 0).then(|| named(false)),
            long_mem(),
            long_cache.clone(),
        ),
        build(
            long_mail(),
            (count > 0).then(|| format!("{count} workers")),
            long_mem(),
            long_cache,
        ),
        build(
            format!("📬 {unread}"),
            (count > 0).then(|| format!("{count}w")),
            compact_mem(),
            compact_cache.clone(),
        ),
        build(format!("📬 {unread}"), None, compact_mem(), compact_cache),
        format!("📬 {unread}"),
    ];
    fit(candidates, width)
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

    fn status() -> Status {
        Status {
            unread: 2,
            workers: vec![
                WorkerState {
                    id: "GUZ-416".into(),
                    state: "running".into(),
                },
                WorkerState {
                    id: "GUZ-417".into(),
                    state: "question".into(),
                },
            ],
            memory: Memory {
                msgs: 132,
                tokens: 4100,
            },
            turn: None,
        }
    }

    fn turn() -> TurnCache {
        TurnCache {
            input: 100,
            read: 12_300,
            write: 400,
            cost_usd: Some(0.0412),
        }
    }

    fn with_turn() -> Status {
        Status {
            turn: Some(turn()),
            ..status()
        }
    }

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
    fn status_line_full_width_with_cache() {
        assert_eq!(
            status_line(&with_turn(), 200),
            "📬 2 unread · workers GUZ-416 running, GUZ-417 question · mem 132 msgs ~4.1k tok · cache 12.3k read 400 write 96% hit $0.04"
        );
    }

    #[test]
    fn status_line_with_cache_never_exceeds_width_and_goes_compact() {
        let status = with_turn();
        assert!((0..120).all(|w| status_line(&status, w).width() <= w));
        assert!(status_line(&status, 40).contains("c96% $0.04"));
        assert_eq!(status_line(&status, 0), "");
    }

    #[test]
    fn status_line_cache_omits_missing_cost() {
        let status = Status {
            turn: Some(TurnCache {
                cost_usd: None,
                ..turn()
            }),
            ..status()
        };
        assert!(status_line(&status, 200).ends_with("96% hit"));
    }

    #[test]
    fn hit_percent_is_zero_without_tokens() {
        assert_eq!(TurnCache::default().hit_percent(), 0);
    }

    #[test]
    fn status_line_full_width() {
        assert_eq!(
            status_line(&status(), 200),
            "📬 2 unread · workers GUZ-416 running, GUZ-417 question · mem 132 msgs ~4.1k tok"
        );
    }

    #[test]
    fn status_line_degrades_and_never_exceeds_width() {
        let status = status();
        let lines: Vec<String> = (0..120).map(|w| status_line(&status, w)).collect();
        assert!(lines.iter().enumerate().all(|(w, l)| l.width() <= w));
        assert!(status_line(&status, 70).contains("workers GUZ-416, GUZ-417"));
        assert!(status_line(&status, 50).contains("2 workers"));
        assert!(status_line(&status, 30).contains("2w"));
        assert_eq!(status_line(&status, 0), "");
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
    fn status_line_without_workers_omits_the_section() {
        let status = Status {
            workers: Vec::new(),
            ..status()
        };
        assert_eq!(
            status_line(&status, 200),
            "📬 2 unread · mem 132 msgs ~4.1k tok"
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
    fn route_input_decides_from_line_and_unread() {
        assert_eq!(route_input(" /inbox ", 0), Route::ReadInbox);
        assert_eq!(route_input("  ", 3), Route::ReadInbox);
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
        ] {
            assert_eq!(route_input(usage, 3), Route::InboxUsage);
        }
        assert_eq!(route_input("/inboxfoo", 0), Route::Send("/inboxfoo".into()));
    }

    #[test]
    fn store_workers_keep_only_this_stores_sessions() {
        let listing = "ralph-p-A\t/s\t\tA\nralph-p-B\t/other\topencode\tB\nralph-p-C\t\t\tC\nralph-D\t/s\nmain\t/s\n";
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
            "main\t/s\nralph-p-GUZ-2\t/s\t\tGUZ-2\nralph-p-GUZ-1\t/s\t\tGUZ-1\nralph-\t/s\t\t\n";
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
        let states = worker_states(&workers, &latest_states(&reports), None);
        assert_eq!(states[0].state, "running");
        assert_eq!(states[1].state, "done");
    }

    #[test]
    fn store_workers_take_the_bare_id_from_the_id_tag() {
        let workers = store_workers("ralph-my-proj-GUZ-1\t/s\topencode\tGUZ-1\n", "/s");
        assert_eq!(
            workers,
            vec![RunningWorker {
                id: "GUZ-1".into(),
                session: "ralph-my-proj-GUZ-1".into(),
                opencode: true
            }]
        );
        let active = HashSet::from([opencode_session_id("GUZ-1")]);
        let states = worker_states(&workers, &HashMap::new(), Some(&active));
        assert_eq!(states[0].state, BUSY_STATE);
    }

    #[test]
    fn orphan_hint_lists_ready_ids_without_a_session() {
        let running = [RunningWorker {
            id: "A".into(),
            session: "ralph-p-A".into(),
            opencode: false,
        }];
        let ready = ["C".to_string(), "A".to_string(), "B".to_string()];
        let hint = orphan_hint(&ready, &running).unwrap();
        assert!(hint.contains(": B, C."), "{hint}");
        assert!(hint.contains("nothing was removed"));
        assert_eq!(orphan_hint(&ready[1..2], &running), None);
        assert_eq!(orphan_hint(&[], &running), None);
    }

    #[test]
    fn store_workers_read_the_harness_tag() {
        let listing = "ralph-p-A\t/s\topencode\tA\nralph-p-B\t/s\t\tB\nralph-p-C\t/s\t\tC\n";
        let opencode: Vec<bool> = store_workers(listing, "/s")
            .iter()
            .map(|worker| worker.opencode)
            .collect();
        assert_eq!(opencode, vec![true, false, false]);
    }

    #[test]
    fn active_sessions_reads_the_session_keys() {
        let active = active_sessions(r#"{"data":{"ses_ralphA":{"type":"running"}}}"#);
        assert_eq!(active, Some(HashSet::from(["ses_ralphA".to_string()])));
        assert_eq!(active_sessions(r#"{"data":{}}"#), Some(HashSet::new()));
        assert_eq!(active_sessions("HTTP 500"), None);
    }

    #[test]
    fn opencode_workers_are_busy_while_their_session_is_active() {
        let worker = |id: &str, opencode| RunningWorker {
            id: id.into(),
            session: format!("ralph-p-{id}"),
            opencode,
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
            worker_states(&workers, &seen, active)
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
    fn memory_size_is_chars_over_four() {
        let lines = vec!["abcdefgh".to_string(), "ééééééé".to_string()];
        assert_eq!(memory_size(2, &lines), Memory { msgs: 2, tokens: 3 });
    }
}
