use crate::snapshot::{
    chat_ref, parse_turns, parse_worker_turns, worker_ref, worker_ref_prefix, Edge, CHAT_REF_PREFIX,
};
use serde_json::{json, Value};
use std::collections::HashSet;

mod cap;

use cap::cap_output;

pub const DEFAULT_CHANGE_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeScope {
    Worker(String),
    WorkerTurn { worker: String, turn: usize },
    WorkerSince { worker: String, turn: usize },
    ChatTurn(usize),
    ChatSince(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnWindow {
    Last(usize),
    Ago(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeQuery {
    Chat(TurnWindow),
    Worker {
        worker: String,
        window: Option<TurnWindow>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeFormat {
    Stat,
    Files,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequest<S = ChangeScope> {
    pub scope: S,
    pub format: ChangeFormat,
    pub paths: Vec<String>,
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRange {
    pub from: String,
    pub to: Option<String>,
}

pub fn diff_args(request: &ChangeRequest, range: &SnapshotRange) -> Vec<String> {
    let mut args: Vec<String> = ["git", "--no-pager", "diff", "--no-color", "--no-ext-diff"]
        .iter()
        .map(|word| word.to_string())
        .collect();
    args.push(format_flag(request.format).to_string());
    args.push(range.from.clone());
    args.extend(range.to.iter().cloned());
    args.push("--".to_string());
    args.extend(request.paths.iter().cloned());
    args
}

fn format_flag(format: ChangeFormat) -> &'static str {
    match format {
        ChangeFormat::Stat => "--stat",
        ChangeFormat::Files => "--name-status",
        ChangeFormat::Patch => "--patch",
    }
}

pub fn spawn_request(worker: &str) -> ChangeRequest {
    stat_request(ChangeScope::Worker(worker.to_string()))
}

pub fn turn_request(worker: &str, turn: u64) -> ChangeRequest {
    stat_request(ChangeScope::WorkerTurn {
        worker: worker.to_string(),
        turn: usize::try_from(turn).unwrap_or(usize::MAX),
    })
}

fn stat_request(scope: ChangeScope) -> ChangeRequest {
    ChangeRequest {
        scope,
        format: ChangeFormat::Stat,
        paths: Vec::new(),
        max_bytes: None,
    }
}

pub fn append_changes(text: &str, blocks: &[String]) -> String {
    std::iter::once(text)
        .chain(blocks.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn scope_label(scope: &ChangeScope) -> String {
    match scope {
        ChangeScope::Worker(worker) => format!("changed in worker {worker} since spawn"),
        ChangeScope::WorkerTurn { worker, turn } => {
            format!("changed in worker {worker} during turn {turn}")
        }
        ChangeScope::WorkerSince { worker, turn } => {
            format!("changed in worker {worker} since turn {turn}")
        }
        ChangeScope::ChatTurn(turn) => format!("changed in chat turn {turn}"),
        ChangeScope::ChatSince(turn) => format!("changed in chat since turn {turn}"),
    }
}

pub fn changes_tool_definition() -> Value {
    json!({
        "name": "changes",
        "description": "Show file changes between turn snapshots as plain text. Turns are counted back from the latest finished turn, so you never need turn numbers. With no worker, shows the chat repo: the latest finished chat turn by default, the net changes of the last N finished turns with `last`, or one finished turn N turns back with `ago` (ago=1 is the latest finished turn, ago=2 the one before). With `worker`, shows that worker's worktree: since spawn by default, or `last`/`ago` over its finished turns. The running turn is never included. Output is capped at max_bytes; narrow with paths or format=stat.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "worker": {"type": "string", "description": "Worker id. Omit for the chat repo."},
                "last": {"type": "integer", "minimum": 1, "description": "Net changes of the last N finished turns. Default 1 without worker. Exclusive with ago."},
                "ago": {"type": "integer", "minimum": 1, "description": "Changes of the one finished turn N turns back; 1 is the latest. Exclusive with last."},
                "format": {"type": "string", "enum": ["stat", "files", "patch"], "description": "stat (default) is a diffstat, files is name and status, patch is the full diff."},
                "paths": {"type": "array", "items": {"type": "string"}, "description": "Limit the diff to these paths."},
                "max_bytes": {"type": "integer", "minimum": 1, "description": "Output cap in bytes. Default 32768."}
            }
        }
    })
}

fn optional_count(args: &Value, key: &str) -> Result<Option<usize>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| format!("Argument '{key}' must be a non-negative integer.")),
    }
}

fn optional_turns(args: &Value, key: &str) -> Result<Option<usize>, String> {
    match optional_count(args, key)? {
        Some(0) => Err(format!("Argument '{key}' must be 1 or more.")),
        count => Ok(count),
    }
}

fn optional_window(args: &Value) -> Result<Option<TurnWindow>, String> {
    match (optional_turns(args, "last")?, optional_turns(args, "ago")?) {
        (Some(_), Some(_)) => Err("Arguments 'last' and 'ago' are mutually exclusive.".into()),
        (Some(count), None) => Ok(Some(TurnWindow::Last(count))),
        (None, Some(count)) => Ok(Some(TurnWindow::Ago(count))),
        (None, None) => Ok(None),
    }
}

fn optional_worker(args: &Value) -> Result<Option<String>, String> {
    match args.get("worker") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(worker)) if !worker.trim().is_empty() => Ok(Some(worker.clone())),
        Some(_) => Err("Argument 'worker' must be a non-empty string.".to_string()),
    }
}

fn parse_format(args: &Value) -> Result<ChangeFormat, String> {
    match args.get("format") {
        None | Some(Value::Null) => Ok(ChangeFormat::Stat),
        Some(Value::String(name)) => match name.as_str() {
            "stat" => Ok(ChangeFormat::Stat),
            "files" => Ok(ChangeFormat::Files),
            "patch" => Ok(ChangeFormat::Patch),
            other => Err(format!(
                "Unknown format '{other}'. Use stat, files or patch."
            )),
        },
        Some(_) => Err("Argument 'format' must be a string.".to_string()),
    }
}

fn parse_paths(args: &Value) -> Result<Vec<String>, String> {
    match args.get("paths") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "Argument 'paths' must be an array of strings.".to_string())
            })
            .collect(),
        Some(_) => Err("Argument 'paths' must be an array of strings.".to_string()),
    }
}

fn parse_max_bytes(args: &Value) -> Result<usize, String> {
    match optional_count(args, "max_bytes")? {
        Some(0) => Err("Argument 'max_bytes' must be a positive integer.".to_string()),
        Some(bytes) => Ok(bytes),
        None => Ok(DEFAULT_CHANGE_BYTES),
    }
}

fn reject_removed_args(args: &Value) -> Result<(), String> {
    match ["turn", "since"].into_iter().find(|key| args.get(key).is_some()) {
        Some(key) => Err(format!(
            "Argument '{key}' was removed; use 'last' (net changes of the last N finished turns) or 'ago' (one finished turn N back, 1 = newest)."
        )),
        None => Ok(()),
    }
}

fn parse_scope(args: &Value) -> Result<ScopeQuery, String> {
    reject_removed_args(args)?;
    let window = optional_window(args)?;
    Ok(match optional_worker(args)? {
        Some(worker) => ScopeQuery::Worker { worker, window },
        None => ScopeQuery::Chat(window.unwrap_or(TurnWindow::Last(1))),
    })
}

pub fn parse_changes_args(args: &Value) -> Result<ChangeRequest<ScopeQuery>, String> {
    Ok(ChangeRequest {
        scope: parse_scope(args)?,
        format: parse_format(args)?,
        paths: parse_paths(args)?,
        max_bytes: Some(parse_max_bytes(args)?),
    })
}

pub fn ref_prefix(scope: &ScopeQuery) -> String {
    match scope {
        ScopeQuery::Worker { worker, .. } => worker_ref_prefix(worker),
        ScopeQuery::Chat(_) => CHAT_REF_PREFIX.to_string(),
    }
}

fn turn_id(turn: u64) -> usize {
    usize::try_from(turn).unwrap_or(usize::MAX)
}

struct RefSet<'a>(HashSet<&'a str>);

impl RefSet<'_> {
    fn require(&self, name: String, missing: impl FnOnce() -> String) -> Result<String, String> {
        if self.0.contains(name.as_str()) {
            Ok(name)
        } else {
            Err(missing())
        }
    }
}

/// Picks the first and last step (oldest first, inclusive) that `window`
/// covers out of `steps` finished steps, counted back from the newest.
fn window_steps(window: TurnWindow, steps: usize) -> Option<(usize, usize)> {
    match window {
        TurnWindow::Last(count) if (1..=steps).contains(&count) => Some((steps - count, steps - 1)),
        TurnWindow::Ago(count) if (1..=steps).contains(&count) => {
            Some((steps - count, steps - count))
        }
        TurnWindow::Last(_) | TurnWindow::Ago(_) => None,
    }
}

fn window_error(window: TurnWindow, steps: usize, owner: &str) -> String {
    let (key, count) = match window {
        TurnWindow::Last(count) => ("last", count),
        TurnWindow::Ago(count) => ("ago", count),
    };
    format!("{key}={count} is more than the {steps} finished turns of {owner}; use {key} {steps} or less")
}

fn chat_range(
    refs: &RefSet,
    ref_list: &str,
    window: TurnWindow,
) -> Result<(ChangeScope, SnapshotRange), String> {
    let finished: Vec<u64> = parse_turns(ref_list)
        .into_iter()
        .filter(|turn| refs.0.contains(chat_ref(*turn, Edge::End).as_str()))
        .collect();
    if finished.is_empty() {
        return Err("no finished chat turn snapshots yet".to_string());
    }
    let (first, last) = window_steps(window, finished.len())
        .map(|(first, last)| (finished[first], finished[last]))
        .ok_or_else(|| window_error(window, finished.len(), "the chat"))?;
    let start = refs.require(chat_ref(first, Edge::Start), || {
        format!("chat turn {first} has no start snapshot")
    })?;
    let scope = if first == last {
        ChangeScope::ChatTurn(turn_id(last))
    } else {
        ChangeScope::ChatSince(turn_id(first))
    };
    Ok((
        scope,
        SnapshotRange {
            from: start,
            to: Some(chat_ref(last, Edge::End)),
        },
    ))
}

fn worker_range(
    refs: &RefSet,
    ref_list: &str,
    worker: &str,
    window: Option<TurnWindow>,
) -> Result<(ChangeScope, SnapshotRange), String> {
    let turns = parse_worker_turns(worker, ref_list);
    let highest = *turns
        .last()
        .ok_or_else(|| format!("no snapshots for worker {worker}"))?;
    let owner = || worker.to_string();
    let (scope, from, to) = match window {
        None => (ChangeScope::Worker(owner()), 0, highest),
        Some(window) => {
            let steps = turns.len() - 1;
            let (first, last) = window_steps(window, steps)
                .ok_or_else(|| window_error(window, steps, &format!("worker {worker}")))?;
            let (from, to) = (turns[first], turns[last + 1]);
            let scope = match window {
                TurnWindow::Ago(_) => ChangeScope::WorkerTurn {
                    worker: owner(),
                    turn: turn_id(to),
                },
                TurnWindow::Last(_) => ChangeScope::WorkerSince {
                    worker: owner(),
                    turn: turn_id(from),
                },
            };
            (scope, from, to)
        }
    };
    let missing = |turn: u64| move || format!("worker {worker} has no snapshot for turn {turn}");
    Ok((
        scope,
        SnapshotRange {
            from: refs.require(worker_ref(worker, from), missing(from))?,
            to: Some(refs.require(worker_ref(worker, to), missing(to))?),
        },
    ))
}

pub fn resolve_range(
    request: &ChangeRequest<ScopeQuery>,
    ref_list: &str,
) -> Result<(ChangeRequest, SnapshotRange), String> {
    let refs = RefSet(ref_list.lines().map(str::trim).collect());
    let (scope, range) = match &request.scope {
        ScopeQuery::Chat(window) => chat_range(&refs, ref_list, *window)?,
        ScopeQuery::Worker { worker, window } => worker_range(&refs, ref_list, worker, *window)?,
    };
    Ok((
        ChangeRequest {
            scope,
            format: request.format,
            paths: request.paths.clone(),
            max_bytes: request.max_bytes,
        },
        range,
    ))
}

pub fn render_changes(request: &ChangeRequest, output: &str) -> String {
    let body = if output.trim().is_empty() {
        "no changes".to_string()
    } else {
        match request.max_bytes {
            Some(max_bytes) => cap_output(request.format, output, max_bytes),
            None => output.to_string(),
        }
    };
    format!("{}\n{}", scope_label(&request.scope), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(paths: &[&str], max_bytes: Option<usize>) -> ChangeRequest {
        ChangeRequest {
            scope: ChangeScope::Worker("GUZ-1".to_string()),
            format: ChangeFormat::Stat,
            paths: paths.iter().map(|path| path.to_string()).collect(),
            max_bytes,
        }
    }

    fn range(to: Option<&str>) -> SnapshotRange {
        SnapshotRange {
            from: "aaa".to_string(),
            to: to.map(str::to_string),
        }
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn diff_args_stat_with_both_ids() {
        let args = diff_args(&request(&[], None), &range(Some("bbb")));
        assert_eq!(
            args,
            strings(&[
                "git",
                "--no-pager",
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--stat",
                "aaa",
                "bbb",
                "--"
            ])
        );
    }

    #[test]
    fn diff_args_paths_follow_separator() {
        let args = diff_args(&request(&["src/a.rs", "-weird"], None), &range(Some("bbb")));
        assert_eq!(
            args[6..],
            strings(&["aaa", "bbb", "--", "src/a.rs", "-weird"])[..]
        );
    }

    #[test]
    fn diff_args_empty_paths_still_end_with_separator() {
        let args = diff_args(&request(&[], None), &range(None));
        assert_eq!(args.last().map(String::as_str), Some("--"));
    }

    #[test]
    fn scope_label_worker_since_spawn() {
        assert_eq!(
            scope_label(&ChangeScope::Worker("GUZ-1".to_string())),
            "changed in worker GUZ-1 since spawn"
        );
    }

    #[test]
    fn scope_label_worker_turn() {
        let scope = ChangeScope::WorkerTurn {
            worker: "GUZ-1".to_string(),
            turn: 2,
        };
        assert_eq!(scope_label(&scope), "changed in worker GUZ-1 during turn 2");
    }

    #[test]
    fn render_empty_output_says_no_changes() {
        let req = request(&[], None);
        assert_eq!(
            render_changes(&req, ""),
            "changed in worker GUZ-1 since spawn\nno changes"
        );
        assert_eq!(
            render_changes(&req, " \n\t\n"),
            "changed in worker GUZ-1 since spawn\nno changes"
        );
    }

    #[test]
    fn render_under_cap_passes_through() {
        let req = request(&[], Some(100));
        assert_eq!(
            render_changes(&req, "M\ta.rs\n"),
            "changed in worker GUZ-1 since spawn\nM\ta.rs\n"
        );
    }

    #[test]
    fn render_without_cap_keeps_everything() {
        let req = request(&[], None);
        let output = "y".repeat(5000);
        let rendered = render_changes(&req, &output);
        assert!(!rendered.contains("omitted"));
        assert!(rendered.ends_with(&output));
    }

    #[test]
    fn render_label_is_never_trimmed() {
        let req = request(&[], Some(2));
        let rendered = render_changes(&req, &"z".repeat(100));
        assert!(rendered.starts_with("changed in worker GUZ-1 since spawn\n"));
    }

    #[test]
    fn spawn_and_turn_requests_are_uncapped_stats() {
        let spawn = spawn_request("GUZ-1");
        assert_eq!(spawn.scope, ChangeScope::Worker("GUZ-1".to_string()));
        assert_eq!(spawn.format, ChangeFormat::Stat);
        assert!(spawn.paths.is_empty());
        assert_eq!(spawn.max_bytes, None);
        let turn = turn_request("GUZ-1", 3);
        assert_eq!(
            turn.scope,
            ChangeScope::WorkerTurn {
                worker: "GUZ-1".to_string(),
                turn: 3
            }
        );
        assert_eq!(turn.format, ChangeFormat::Stat);
        assert_eq!(turn.max_bytes, None);
    }

    #[test]
    fn append_changes_separates_blocks_with_blank_lines() {
        let blocks = vec!["a\n 1 file".to_string(), "b\nno changes".to_string()];
        assert_eq!(
            append_changes("done", &blocks),
            "done\n\na\n 1 file\n\nb\nno changes"
        );
    }

    #[test]
    fn append_changes_without_blocks_keeps_the_text() {
        assert_eq!(append_changes("done", &[]), "done");
    }

    fn chat_req(
        scope: ChangeScope,
        format: ChangeFormat,
        max_bytes: Option<usize>,
    ) -> ChangeRequest {
        ChangeRequest {
            scope,
            format,
            paths: Vec::new(),
            max_bytes,
        }
    }

    fn parsed(args: Value) -> Result<ChangeRequest<ScopeQuery>, String> {
        parse_changes_args(&args)
    }

    #[test]
    fn diff_args_files_and_patch_flags() {
        let files = chat_req(ChangeScope::ChatTurn(1), ChangeFormat::Files, None);
        assert!(diff_args(&files, &range(None)).contains(&"--name-status".to_string()));
        let patch = chat_req(ChangeScope::ChatTurn(1), ChangeFormat::Patch, None);
        assert!(diff_args(&patch, &range(None)).contains(&"--patch".to_string()));
    }

    #[test]
    fn chat_and_worker_since_labels() {
        assert_eq!(
            scope_label(&ChangeScope::ChatTurn(4)),
            "changed in chat turn 4"
        );
        assert_eq!(
            scope_label(&ChangeScope::ChatSince(2)),
            "changed in chat since turn 2"
        );
        let since = ChangeScope::WorkerSince {
            worker: "GUZ-1".to_string(),
            turn: 3,
        };
        assert_eq!(scope_label(&since), "changed in worker GUZ-1 since turn 3");
    }

    #[test]
    fn parse_defaults_to_latest_chat_turn_stat() {
        let request = parsed(json!({})).unwrap();
        assert_eq!(request.scope, ScopeQuery::Chat(TurnWindow::Last(1)));
        assert_eq!(request.format, ChangeFormat::Stat);
        assert!(request.paths.is_empty());
        assert_eq!(request.max_bytes, Some(DEFAULT_CHANGE_BYTES));
        assert_eq!(parsed(Value::Null).unwrap(), request);
    }

    #[test]
    fn parse_scope_mapping() {
        let scope = |args: Value| parsed(args).unwrap().scope;
        assert_eq!(
            scope(json!({"last": 3})),
            ScopeQuery::Chat(TurnWindow::Last(3))
        );
        assert_eq!(
            scope(json!({"ago": 2})),
            ScopeQuery::Chat(TurnWindow::Ago(2))
        );
        assert_eq!(
            scope(json!({"worker": "A"})),
            ScopeQuery::Worker {
                worker: "A".to_string(),
                window: None
            }
        );
        assert_eq!(
            scope(json!({"worker": "A", "ago": 1})),
            ScopeQuery::Worker {
                worker: "A".to_string(),
                window: Some(TurnWindow::Ago(1))
            }
        );
        assert_eq!(
            scope(json!({"worker": "A", "last": 2})),
            ScopeQuery::Worker {
                worker: "A".to_string(),
                window: Some(TurnWindow::Last(2))
            }
        );
    }

    #[test]
    fn parse_rejects_bad_arguments() {
        for args in [
            json!({"last": 1, "ago": 1}),
            json!({"worker": "A", "last": 2, "ago": 1}),
            json!({"last": 0}),
            json!({"ago": 0}),
            json!({"ago": -1}),
            json!({"last": "1"}),
            json!({"worker": ""}),
            json!({"worker": 5}),
            json!({"format": "diff"}),
            json!({"format": 1}),
            json!({"paths": "a"}),
            json!({"paths": ["a", 1]}),
            json!({"max_bytes": 0}),
            json!({"max_bytes": "x"}),
        ] {
            assert!(parsed(args.clone()).is_err(), "{args}");
        }
        assert_eq!(
            parsed(json!({"last": 1, "ago": 1})).unwrap_err(),
            "Arguments 'last' and 'ago' are mutually exclusive."
        );
        assert_eq!(
            parsed(json!({"turn": 32})).unwrap_err(),
            "Argument 'turn' was removed; use 'last' (net changes of the last N finished turns) or 'ago' (one finished turn N back, 1 = newest)."
        );
        assert!(parsed(json!({"worker": "A", "since": 0}))
            .unwrap_err()
            .starts_with("Argument 'since' was removed; use 'last'"));
        assert_eq!(
            parsed(json!({"ago": 0})).unwrap_err(),
            "Argument 'ago' must be 1 or more."
        );
    }

    #[test]
    fn parse_format_paths_and_max_bytes() {
        let request =
            parsed(json!({"format": "patch", "paths": ["a.rs", "b"], "max_bytes": 500})).unwrap();
        assert_eq!(request.format, ChangeFormat::Patch);
        assert_eq!(request.paths, strings(&["a.rs", "b"]));
        assert_eq!(request.max_bytes, Some(500));
        assert_eq!(
            parsed(json!({"format": "files"})).unwrap().format,
            ChangeFormat::Files
        );
    }

    #[test]
    fn ref_prefix_follows_scope() {
        assert_eq!(
            ref_prefix(&ScopeQuery::Chat(TurnWindow::Last(1))),
            "refs/ralph/chat/"
        );
        assert_eq!(
            ref_prefix(&ScopeQuery::Worker {
                worker: "A".to_string(),
                window: Some(TurnWindow::Ago(1))
            }),
            "refs/ralph/workers/A/"
        );
    }

    #[test]
    fn window_steps_counts_back_from_the_newest() {
        assert_eq!(window_steps(TurnWindow::Last(1), 3), Some((2, 2)));
        assert_eq!(window_steps(TurnWindow::Last(3), 3), Some((0, 2)));
        assert_eq!(window_steps(TurnWindow::Ago(1), 3), Some((2, 2)));
        assert_eq!(window_steps(TurnWindow::Ago(3), 3), Some((0, 0)));
        assert_eq!(window_steps(TurnWindow::Last(4), 3), None);
        assert_eq!(window_steps(TurnWindow::Ago(4), 3), None);
        assert_eq!(window_steps(TurnWindow::Last(0), 3), None);
        assert_eq!(window_steps(TurnWindow::Ago(1), 0), None);
    }

    const CHAT_REFS: &str = "refs/ralph/chat/turn-30/start\nrefs/ralph/chat/turn-30/end\nrefs/ralph/chat/turn-31/start\nrefs/ralph/chat/turn-31/end\nrefs/ralph/chat/turn-32/start\nrefs/ralph/chat/turn-32/end\nrefs/ralph/chat/turn-33/start\n";
    const WORKER_REFS: &str = "refs/ralph/workers/A/turn-0\nrefs/ralph/workers/A/turn-1\nrefs/ralph/workers/A/turn-2\nrefs/ralph/workers/B/turn-5\n";

    fn resolve_query(
        scope: ScopeQuery,
        refs: &str,
    ) -> Result<(ChangeRequest, SnapshotRange), String> {
        let query = ChangeRequest {
            scope,
            format: ChangeFormat::Patch,
            paths: strings(&["src"]),
            max_bytes: Some(10),
        };
        resolve_range(&query, refs)
    }

    fn resolve_chat(window: TurnWindow) -> Result<(ChangeScope, SnapshotRange), String> {
        resolve_query(ScopeQuery::Chat(window), CHAT_REFS)
            .map(|(request, range)| (request.scope, range))
    }

    fn resolve_worker(
        worker: &str,
        window: Option<TurnWindow>,
    ) -> Result<(ChangeScope, SnapshotRange), String> {
        let scope = ScopeQuery::Worker {
            worker: worker.to_string(),
            window,
        };
        resolve_query(scope, WORKER_REFS).map(|(request, range)| (request.scope, range))
    }

    fn span(from: &str, to: &str) -> SnapshotRange {
        SnapshotRange {
            from: from.to_string(),
            to: Some(to.to_string()),
        }
    }

    #[test]
    fn resolve_chat_default_is_latest_finished_turn() {
        let (request, range) =
            resolve_query(ScopeQuery::Chat(TurnWindow::Last(1)), CHAT_REFS).unwrap();
        assert_eq!(
            request,
            ChangeRequest {
                scope: ChangeScope::ChatTurn(32),
                format: ChangeFormat::Patch,
                paths: strings(&["src"]),
                max_bytes: Some(10),
            }
        );
        assert_eq!(
            range,
            span(
                "refs/ralph/chat/turn-32/start",
                "refs/ralph/chat/turn-32/end"
            )
        );
    }

    #[test]
    fn resolve_chat_ago_picks_one_finished_turn() {
        assert_eq!(
            resolve_chat(TurnWindow::Ago(1)).unwrap(),
            (
                ChangeScope::ChatTurn(32),
                span(
                    "refs/ralph/chat/turn-32/start",
                    "refs/ralph/chat/turn-32/end"
                )
            )
        );
        assert_eq!(
            resolve_chat(TurnWindow::Ago(3)).unwrap(),
            (
                ChangeScope::ChatTurn(30),
                span(
                    "refs/ralph/chat/turn-30/start",
                    "refs/ralph/chat/turn-30/end"
                )
            )
        );
    }

    #[test]
    fn resolve_chat_last_spans_finished_turns() {
        assert_eq!(
            resolve_chat(TurnWindow::Last(2)).unwrap(),
            (
                ChangeScope::ChatSince(31),
                span(
                    "refs/ralph/chat/turn-31/start",
                    "refs/ralph/chat/turn-32/end"
                )
            )
        );
    }

    #[test]
    fn resolve_chat_window_errors() {
        assert_eq!(
            resolve_chat(TurnWindow::Last(4)).unwrap_err(),
            "last=4 is more than the 3 finished turns of the chat; use last 3 or less"
        );
        assert_eq!(
            resolve_chat(TurnWindow::Ago(5)).unwrap_err(),
            "ago=5 is more than the 3 finished turns of the chat; use ago 3 or less"
        );
        let unfinished = "refs/ralph/chat/turn-1/start\n";
        assert_eq!(
            resolve_query(ScopeQuery::Chat(TurnWindow::Last(1)), unfinished).unwrap_err(),
            "no finished chat turn snapshots yet"
        );
        assert_eq!(
            resolve_query(
                ScopeQuery::Chat(TurnWindow::Last(1)),
                "refs/ralph/chat/turn-4/end\n"
            )
            .unwrap_err(),
            "chat turn 4 has no start snapshot"
        );
    }

    #[test]
    fn resolve_worker_ranges() {
        let worker = || "A".to_string();
        assert_eq!(
            resolve_worker("A", None).unwrap(),
            (
                ChangeScope::Worker(worker()),
                span("refs/ralph/workers/A/turn-0", "refs/ralph/workers/A/turn-2")
            )
        );
        assert_eq!(
            resolve_worker("A", Some(TurnWindow::Ago(1))).unwrap(),
            (
                ChangeScope::WorkerTurn {
                    worker: worker(),
                    turn: 2
                },
                span("refs/ralph/workers/A/turn-1", "refs/ralph/workers/A/turn-2")
            )
        );
        assert_eq!(
            resolve_worker("A", Some(TurnWindow::Ago(2))).unwrap().1,
            span("refs/ralph/workers/A/turn-0", "refs/ralph/workers/A/turn-1")
        );
        assert_eq!(
            resolve_worker("A", Some(TurnWindow::Last(2))).unwrap(),
            (
                ChangeScope::WorkerSince {
                    worker: worker(),
                    turn: 0
                },
                span("refs/ralph/workers/A/turn-0", "refs/ralph/workers/A/turn-2")
            )
        );
    }

    #[test]
    fn resolve_worker_errors() {
        assert_eq!(
            resolve_worker("Z", None).unwrap_err(),
            "no snapshots for worker Z"
        );
        assert_eq!(
            resolve_worker("A", Some(TurnWindow::Ago(3))).unwrap_err(),
            "ago=3 is more than the 2 finished turns of worker A; use ago 2 or less"
        );
        assert_eq!(
            resolve_worker("B", Some(TurnWindow::Last(1))).unwrap_err(),
            "last=1 is more than the 0 finished turns of worker B; use last 0 or less"
        );
        assert_eq!(
            resolve_worker("B", None).unwrap_err(),
            "worker B has no snapshot for turn 0"
        );
    }

    #[test]
    fn tool_definition_names_the_arguments() {
        let definition = changes_tool_definition();
        assert_eq!(definition["name"], "changes");
        let properties = definition["inputSchema"]["properties"].as_object().unwrap();
        for key in ["worker", "last", "ago", "format", "paths", "max_bytes"] {
            assert!(properties.contains_key(key), "{key}");
        }
    }
}
