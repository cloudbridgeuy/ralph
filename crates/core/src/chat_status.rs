use crate::worker::{session_name, Report};
use serde::Deserialize;
use std::collections::HashMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const DEFAULT_STATE: &str = "running";
const INBOX_COMMAND: &str = "/inbox";
const SEPARATOR: &str = " · ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerState {
    pub id: String,
    pub state: String,
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
    match line.trim() {
        INBOX_COMMAND => Route::ReadInbox,
        "" if unread > 0 => Route::ReadInbox,
        "" => Route::Ignore,
        message => Route::Send(message.to_string()),
    }
}

/// Worker ids from `tmux list-sessions -F` lines of `<session>\t<@ralph_store>`,
/// keeping only `ralph-*` sessions tagged with `store`.
pub fn store_worker_ids(tmux_sessions: &str, store: &str) -> Vec<String> {
    let prefix = session_name("");
    let mut ids: Vec<String> = tmux_sessions
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(_, tag)| tag.trim() == store)
        .filter_map(|(name, _)| name.trim().strip_prefix(&prefix))
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    ids.sort();
    ids
}

pub fn latest_states(reports: &[Report]) -> HashMap<String, String> {
    reports
        .iter()
        .map(|r| (r.worker.clone(), r.status.clone()))
        .collect()
}

pub fn worker_states(ids: &[String], seen: &HashMap<String, String>) -> Vec<WorkerState> {
    ids.iter()
        .map(|id| WorkerState {
            id: id.clone(),
            state: seen
                .get(id)
                .cloned()
                .unwrap_or_else(|| DEFAULT_STATE.to_string()),
        })
        .collect()
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
            items.push("Ctrl+O editor".to_string());
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
            "Enter send · Ctrl+J newline · /inbox or Enter on empty prompt reads 2 reports · Ctrl+O editor · Ctrl+D quit\n"
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
    }

    #[test]
    fn store_worker_ids_keep_only_this_stores_sessions() {
        let listing = "ralph-A\t/s\nralph-B\t/other\nralph-C\t\nralph-D\nmain\t/s\n";
        assert_eq!(store_worker_ids(listing, "/s"), vec!["A"]);
        assert_eq!(store_worker_ids(listing, "/other"), vec!["B"]);
        assert!(store_worker_ids("", "/s").is_empty());
    }

    #[test]
    fn workers_come_from_prefixed_sessions_with_last_known_state() {
        let listing = "main\t/s\nralph-GUZ-2\t/s\nralph-GUZ-1\t/s\nralph-\t/s\n";
        let ids = store_worker_ids(listing, "/s");
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
        let states = worker_states(&ids, &latest_states(&reports));
        assert_eq!(states[0].state, "running");
        assert_eq!(states[1].state, "done");
    }

    #[test]
    fn memory_size_is_chars_over_four() {
        let lines = vec!["abcdefgh".to_string(), "ééééééé".to_string()];
        assert_eq!(memory_size(2, &lines), Memory { msgs: 2, tokens: 3 });
    }
}
