use crate::snapshot::{
    chat_ref, parse_turns, parse_worker_turns, worker_ref, worker_ref_prefix, Edge, CHAT_REF_PREFIX,
};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const DEFAULT_CHANGE_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeScope {
    Worker(String),
    WorkerTurn { worker: String, turn: usize },
    WorkerSince { worker: String, turn: usize },
    ChatTurn(usize),
    ChatSince(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeQuery {
    ChatLatest,
    Exact(ChangeScope),
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
        "description": "Show file changes between turn snapshots as plain text. With no worker, shows the chat repo: the latest finished chat turn by default, one turn with `turn`, or everything since a turn with `since`. With `worker`, shows that worker's worktree: since spawn by default, one turn with `turn` (1 or more), or since a turn with `since`. Changes after the latest snapshot are not shown. Output is capped at max_bytes; narrow with paths or format=stat.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "worker": {"type": "string", "description": "Worker id. Omit for the chat repo."},
                "turn": {"type": "integer", "minimum": 0, "description": "Show one turn. Exclusive with since."},
                "since": {"type": "integer", "minimum": 0, "description": "Show everything since this turn. Exclusive with turn."},
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

fn parse_scope(args: &Value) -> Result<ScopeQuery, String> {
    let worker = optional_worker(args)?;
    let turn = optional_count(args, "turn")?;
    let since = optional_count(args, "since")?;
    match (worker, turn, since) {
        (_, Some(_), Some(_)) => Err("Arguments 'turn' and 'since' are mutually exclusive.".into()),
        (None, Some(turn), None) => Ok(ScopeQuery::Exact(ChangeScope::ChatTurn(turn))),
        (None, None, Some(turn)) => Ok(ScopeQuery::Exact(ChangeScope::ChatSince(turn))),
        (None, None, None) => Ok(ScopeQuery::ChatLatest),
        (Some(worker), Some(0), None) => Err(format!(
            "Worker {worker} turns start at 1; use since=0 for everything since turn 0."
        )),
        (Some(worker), Some(turn), None) => {
            Ok(ScopeQuery::Exact(ChangeScope::WorkerTurn { worker, turn }))
        }
        (Some(worker), None, Some(turn)) => {
            Ok(ScopeQuery::Exact(ChangeScope::WorkerSince { worker, turn }))
        }
        (Some(worker), None, None) => Ok(ScopeQuery::Exact(ChangeScope::Worker(worker))),
    }
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
        ScopeQuery::Exact(
            ChangeScope::Worker(worker)
            | ChangeScope::WorkerTurn { worker, .. }
            | ChangeScope::WorkerSince { worker, .. },
        ) => worker_ref_prefix(worker),
        ScopeQuery::ChatLatest
        | ScopeQuery::Exact(ChangeScope::ChatTurn(_) | ChangeScope::ChatSince(_)) => {
            CHAT_REF_PREFIX.to_string()
        }
    }
}

fn turn_id(turn: usize) -> u64 {
    u64::try_from(turn).unwrap_or(u64::MAX)
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

fn latest_chat_turn(refs: &RefSet, ref_list: &str) -> Result<usize, String> {
    parse_turns(ref_list)
        .into_iter()
        .rev()
        .find(|turn| refs.0.contains(chat_ref(*turn, Edge::End).as_str()))
        .and_then(|turn| usize::try_from(turn).ok())
        .ok_or_else(|| "no finished chat turn snapshots yet".to_string())
}

fn chat_range(
    refs: &RefSet,
    ref_list: &str,
    from: Option<usize>,
    to: Option<usize>,
) -> Result<(usize, SnapshotRange), String> {
    let last = match to {
        Some(turn) => turn,
        None => latest_chat_turn(refs, ref_list)?,
    };
    let first = from.unwrap_or(last);
    let start = refs.require(chat_ref(turn_id(first), Edge::Start), || {
        format!("chat turn {first} has no start snapshot")
    })?;
    let end = refs.require(chat_ref(turn_id(last), Edge::End), || {
        format!("chat turn {last} has no end snapshot yet")
    })?;
    Ok((
        last,
        SnapshotRange {
            from: start,
            to: Some(end),
        },
    ))
}

fn worker_range(
    refs: &RefSet,
    ref_list: &str,
    worker: &str,
    from: usize,
    to: Option<usize>,
) -> Result<SnapshotRange, String> {
    let highest = parse_worker_turns(worker, ref_list)
        .into_iter()
        .max()
        .ok_or_else(|| format!("no snapshots for worker {worker}"))?;
    let last = to.unwrap_or_else(|| usize::try_from(highest).unwrap_or(usize::MAX));
    let missing = |turn: usize| move || format!("worker {worker} has no snapshot for turn {turn}");
    Ok(SnapshotRange {
        from: refs.require(worker_ref(worker, turn_id(from)), missing(from))?,
        to: Some(refs.require(worker_ref(worker, turn_id(last)), missing(last))?),
    })
}

pub fn resolve_range(
    request: &ChangeRequest<ScopeQuery>,
    ref_list: &str,
) -> Result<(ChangeRequest, SnapshotRange), String> {
    let refs = RefSet(ref_list.lines().map(str::trim).collect());
    let (scope, range) = match &request.scope {
        ScopeQuery::ChatLatest => {
            let (turn, range) = chat_range(&refs, ref_list, None, None)?;
            (ChangeScope::ChatTurn(turn), range)
        }
        ScopeQuery::Exact(scope) => (scope.clone(), exact_range(scope, &refs, ref_list)?),
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

fn exact_range(
    scope: &ChangeScope,
    refs: &RefSet,
    ref_list: &str,
) -> Result<SnapshotRange, String> {
    match scope {
        ChangeScope::ChatTurn(turn) => {
            chat_range(refs, ref_list, Some(*turn), Some(*turn)).map(|(_, range)| range)
        }
        ChangeScope::ChatSince(turn) => {
            chat_range(refs, ref_list, Some(*turn), None).map(|(_, range)| range)
        }
        ChangeScope::Worker(worker) => worker_range(refs, ref_list, worker, 0, None),
        ChangeScope::WorkerSince { worker, turn } => {
            worker_range(refs, ref_list, worker, *turn, None)
        }
        ChangeScope::WorkerTurn { worker, turn } => {
            worker_range(refs, ref_list, worker, turn.saturating_sub(1), Some(*turn))
        }
    }
}

fn is_stat_summary(line: &str) -> bool {
    line.contains(" changed") && (line.contains("file changed") || line.contains("files changed"))
}

fn omitted_files(format: ChangeFormat, omitted: &str) -> usize {
    omitted
        .lines()
        .filter(|line| match format {
            ChangeFormat::Patch => line.starts_with("diff --git "),
            ChangeFormat::Files => !line.trim().is_empty(),
            ChangeFormat::Stat => !line.trim().is_empty() && !is_stat_summary(line),
        })
        .count()
}

fn cap_footer(omitted_bytes: usize, files: usize) -> String {
    format!(
        "… {omitted_bytes} bytes omitted ({files} files not shown); narrow with paths or format=stat"
    )
}

fn char_floor(text: &str, limit: usize) -> usize {
    (0..=limit.min(text.len()))
        .rev()
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(0)
}

fn kept_prefix(output: &str, budget: usize) -> usize {
    let whole_lines = output
        .split_inclusive('\n')
        .scan(0usize, |used, line| {
            *used += line.len();
            Some(*used)
        })
        .take_while(|used| *used <= budget)
        .last();
    whole_lines.unwrap_or_else(|| {
        let first_line = output.find('\n').unwrap_or(output.len());
        char_floor(output, budget.min(first_line))
    })
}

fn cap_output(format: ChangeFormat, output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }
    let reserve = cap_footer(output.len(), omitted_files(format, output)).len() + 1;
    let keep = kept_prefix(output, max_bytes.saturating_sub(reserve));
    let (kept, omitted) = output.split_at(keep);
    let footer = cap_footer(omitted.len(), omitted_files(format, omitted));
    if kept.is_empty() {
        footer
    } else {
        format!("{}\n{footer}", kept.trim_end_matches('\n'))
    }
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
        assert_eq!(request.scope, ScopeQuery::ChatLatest);
        assert_eq!(request.format, ChangeFormat::Stat);
        assert!(request.paths.is_empty());
        assert_eq!(request.max_bytes, Some(DEFAULT_CHANGE_BYTES));
        assert_eq!(parsed(Value::Null).unwrap(), request);
    }

    #[test]
    fn parse_scope_mapping() {
        let scope = |args: Value| match parsed(args).unwrap().scope {
            ScopeQuery::Exact(scope) => scope,
            ScopeQuery::ChatLatest => panic!("expected an exact scope"),
        };
        assert_eq!(scope(json!({"turn": 3})), ChangeScope::ChatTurn(3));
        assert_eq!(scope(json!({"since": 0})), ChangeScope::ChatSince(0));
        assert_eq!(
            scope(json!({"worker": "A"})),
            ChangeScope::Worker("A".to_string())
        );
        assert_eq!(
            scope(json!({"worker": "A", "turn": 2})),
            ChangeScope::WorkerTurn {
                worker: "A".to_string(),
                turn: 2
            }
        );
        assert_eq!(
            scope(json!({"worker": "A", "since": 1})),
            ChangeScope::WorkerSince {
                worker: "A".to_string(),
                turn: 1
            }
        );
    }

    #[test]
    fn parse_rejects_bad_arguments() {
        for args in [
            json!({"turn": 1, "since": 1}),
            json!({"worker": "A", "turn": 0}),
            json!({"turn": -1}),
            json!({"turn": "1"}),
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
        assert_eq!(ref_prefix(&ScopeQuery::ChatLatest), "refs/ralph/chat/");
        assert_eq!(
            ref_prefix(&ScopeQuery::Exact(ChangeScope::ChatSince(1))),
            "refs/ralph/chat/"
        );
        assert_eq!(
            ref_prefix(&ScopeQuery::Exact(ChangeScope::Worker("A".to_string()))),
            "refs/ralph/workers/A/"
        );
    }

    const CHAT_REFS: &str = "refs/ralph/chat/turn-1/start\nrefs/ralph/chat/turn-1/end\nrefs/ralph/chat/turn-2/start\nrefs/ralph/chat/turn-2/end\nrefs/ralph/chat/turn-3/start\n";
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

    fn resolve(scope: ChangeScope, refs: &str) -> Result<(ChangeScope, SnapshotRange), String> {
        resolve_query(ScopeQuery::Exact(scope), refs).map(|(request, range)| (request.scope, range))
    }

    fn span(from: &str, to: &str) -> SnapshotRange {
        SnapshotRange {
            from: from.to_string(),
            to: Some(to.to_string()),
        }
    }

    #[test]
    fn resolve_chat_turn_and_latest() {
        assert_eq!(
            resolve(ChangeScope::ChatTurn(1), CHAT_REFS).unwrap().1,
            span("refs/ralph/chat/turn-1/start", "refs/ralph/chat/turn-1/end")
        );
        let (request, range) = resolve_query(ScopeQuery::ChatLatest, CHAT_REFS).unwrap();
        assert_eq!(
            request,
            ChangeRequest {
                scope: ChangeScope::ChatTurn(2),
                format: ChangeFormat::Patch,
                paths: strings(&["src"]),
                max_bytes: Some(10),
            }
        );
        assert_eq!(
            range,
            span("refs/ralph/chat/turn-2/start", "refs/ralph/chat/turn-2/end")
        );
    }

    #[test]
    fn resolve_chat_since_ends_at_highest_end_ref() {
        assert_eq!(
            resolve(ChangeScope::ChatSince(1), CHAT_REFS).unwrap().1,
            span("refs/ralph/chat/turn-1/start", "refs/ralph/chat/turn-2/end")
        );
    }

    #[test]
    fn resolve_chat_missing_refs_error() {
        assert_eq!(
            resolve(ChangeScope::ChatTurn(3), CHAT_REFS).unwrap_err(),
            "chat turn 3 has no end snapshot yet"
        );
        assert_eq!(
            resolve(ChangeScope::ChatTurn(9), CHAT_REFS).unwrap_err(),
            "chat turn 9 has no start snapshot"
        );
        assert!(resolve(ChangeScope::ChatSince(7), CHAT_REFS).is_err());
        assert!(resolve_query(ScopeQuery::ChatLatest, "").is_err());
        assert!(resolve_query(ScopeQuery::ChatLatest, "refs/ralph/chat/turn-1/start\n").is_err());
    }

    #[test]
    fn resolve_worker_ranges() {
        let worker = || "A".to_string();
        assert_eq!(
            resolve(ChangeScope::Worker(worker()), WORKER_REFS)
                .unwrap()
                .1,
            span("refs/ralph/workers/A/turn-0", "refs/ralph/workers/A/turn-2")
        );
        assert_eq!(
            resolve(
                ChangeScope::WorkerSince {
                    worker: worker(),
                    turn: 1
                },
                WORKER_REFS
            )
            .unwrap()
            .1,
            span("refs/ralph/workers/A/turn-1", "refs/ralph/workers/A/turn-2")
        );
        assert_eq!(
            resolve(
                ChangeScope::WorkerTurn {
                    worker: worker(),
                    turn: 2
                },
                WORKER_REFS
            )
            .unwrap()
            .1,
            span("refs/ralph/workers/A/turn-1", "refs/ralph/workers/A/turn-2")
        );
    }

    #[test]
    fn resolve_worker_missing_refs_error() {
        assert_eq!(
            resolve(ChangeScope::Worker("Z".to_string()), WORKER_REFS).unwrap_err(),
            "no snapshots for worker Z"
        );
        assert!(resolve(
            ChangeScope::WorkerTurn {
                worker: "A".to_string(),
                turn: 3
            },
            WORKER_REFS
        )
        .is_err());
        assert!(resolve(
            ChangeScope::WorkerSince {
                worker: "B".to_string(),
                turn: 0
            },
            WORKER_REFS
        )
        .is_err());
    }

    fn capped(format: ChangeFormat, output: &str, max_bytes: usize) -> String {
        let rendered = render_changes(
            &chat_req(ChangeScope::ChatTurn(1), format, Some(max_bytes)),
            output,
        );
        rendered
            .split_once('\n')
            .map(|(_, body)| body.to_string())
            .unwrap()
    }

    #[test]
    fn cap_under_limit_passes_through() {
        let output = "M\ta.rs\nM\tb.rs\n";
        assert_eq!(capped(ChangeFormat::Files, output, output.len()), output);
    }

    #[test]
    fn cap_keeps_whole_lines_and_counts_omitted_files() {
        let output = "M\ta.rs\nM\tb.rs\nM\tc.rs\nM\td.rs\n".repeat(5);
        let body = capped(ChangeFormat::Files, &output, 120);
        assert!(body.len() <= 120, "{}", body.len());
        let (kept, footer) = body.rsplit_once('\n').unwrap();
        assert!(kept.lines().all(|line| line.starts_with("M\t")));
        assert!(output.starts_with(kept));
        let omitted = output.len() - kept.len() - 1;
        let files = output[kept.len() + 1..].lines().count();
        assert_eq!(
            footer,
            format!(
                "… {omitted} bytes omitted ({files} files not shown); narrow with paths or format=stat"
            )
        );
    }

    #[test]
    fn cap_counts_patch_files_by_diff_headers() {
        let file = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";
        let output = file.repeat(6);
        let body = capped(ChangeFormat::Patch, &output, 200);
        assert!(body.len() <= 200);
        let shown = body.matches("diff --git ").count();
        let footer = body.lines().last().unwrap();
        assert!(
            footer.contains(&format!("({} files not shown)", 6 - shown)),
            "{footer}"
        );
    }

    #[test]
    fn cap_stat_footer_excludes_summary_line() {
        let output = format!(
            "{} 6 files changed, 6 insertions(+), 6 deletions(-)\n",
            " f.rs | 2 +-\n".repeat(6)
        );
        let body = capped(ChangeFormat::Stat, &output, 125);
        let footer = body.lines().last().unwrap();
        assert!(footer.contains("(3 files not shown)"), "{footer}");
        assert!(body.len() <= 125);
        assert!(body.starts_with(" f.rs | 2 +-\n"));
    }

    #[test]
    fn cap_cuts_oversized_first_line_at_char_boundary() {
        let output = format!("{}\nsecond\n", "é".repeat(100));
        let body = capped(ChangeFormat::Patch, &output, 150);
        let kept = body.lines().next().unwrap();
        assert!(!kept.is_empty());
        assert!(kept.chars().all(|c| c == 'é'));
        assert!(body.len() <= 150);
    }

    #[test]
    fn cap_tiny_limit_returns_only_the_footer() {
        let body = capped(ChangeFormat::Files, "M\ta.rs\nM\tb.rs\n", 5);
        assert!(body.starts_with("… 14 bytes omitted (2 files not shown)"));
    }

    #[test]
    fn tool_definition_names_the_arguments() {
        let definition = changes_tool_definition();
        assert_eq!(definition["name"], "changes");
        let properties = definition["inputSchema"]["properties"].as_object().unwrap();
        for key in ["worker", "turn", "since", "format", "paths", "max_bytes"] {
            assert!(properties.contains_key(key), "{key}");
        }
    }
}
