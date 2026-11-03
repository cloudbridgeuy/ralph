use crate::chat_memory::{MemoryCall, Tool, ViewSplit};
use crate::worker::{shell_quote, Report};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const THREADS_DIR: &str = "threads";
pub const THREAD_FILE: &str = "thread.json";
pub const MAIN_PREFIX_LINES: usize = 100;
pub const MAX_NAME_CHARS: usize = 32;
pub const THREAD_REPORT_PREFIX: &str = "thread:";
pub const MERGED_STATUS: &str = "merged";
pub const MAX_DIGEST_BYTES: usize = 2048;
pub const MAX_MERGE_ATTEMPTS: usize = 3;
const MAIN: &str = "main";
pub const FORK_USAGE: &str =
    "usage: /fork <name> opens thread <name> in a new Rex block next to this chat.";
pub const FORK_IN_THREAD: &str = "threads fork from main; run /fork in main's chat.";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThreadName(String);

impl ThreadName {
    pub fn parse(s: &str) -> Result<ThreadName, String> {
        let valid = (1..=MAX_NAME_CHARS).contains(&s.len())
            && !s.starts_with('-')
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid {
            return Err(format!(
                "'{s}' is not a thread name. Use 1 to {MAX_NAME_CHARS} lowercase letters, digits or dashes, not starting with a dash."
            ));
        }
        if s == MAIN {
            return Err("'main' is the main chat, not a thread name.".to_string());
        }
        Ok(ThreadName(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ThreadName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMeta {
    pub base: usize,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged: Option<Merged>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parked: Option<Parked>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merged {
    pub at: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parked {
    pub at: String,
}

pub fn merged_check(name: &ThreadName, meta: &ThreadMeta) -> Result<(), String> {
    match meta.merged {
        None => Ok(()),
        Some(_) => Err(format!(
            "thread {name} is merged into main and stays read-only; `ralph chat --thread {name} history` still reads it."
        )),
    }
}

pub fn open_check(name: &ThreadName, meta: &ThreadMeta) -> Result<(), String> {
    merged_check(name, meta)?;
    match meta.parked {
        None => Ok(()),
        Some(_) => Err(format!(
            "thread {name} is parked; `ralph thread revive {name}` opens it again."
        )),
    }
}

pub fn revive_check(name: &ThreadName, meta: &ThreadMeta) -> Result<(), String> {
    merged_check(name, meta)?;
    match meta.parked {
        Some(_) => Ok(()),
        None => Err(format!("thread {name} is not parked.")),
    }
}

pub fn pull_message(name: &ThreadName, from: usize, to: usize) -> String {
    match to.saturating_sub(from) {
        0 => format!("thread {name} is already at main's end (#{to})."),
        1 => format!("pulled thread {name}: fork point #{from} -> #{to}, 1 new main message."),
        new => {
            format!("pulled thread {name}: fork point #{from} -> #{to}, {new} new main messages.")
        }
    }
}

pub fn refuse_thread_report(id: &str) -> Result<(), String> {
    if !id.starts_with(THREAD_REPORT_PREFIX) {
        return Ok(());
    }
    Err(format!(
        "{id} is a merged thread's digest, not a worker; there is nothing to reply to, nudge or close."
    ))
}

pub fn merge_prompt(name: &ThreadName, base: usize, lines: &[String]) -> String {
    format!(
        "Write the merge digest of chat thread {name}, forked from main at #{base}. \
         Main reads it once, as one inbox message, instead of the whole thread.\n\
         Keep the decisions, results and open items; drop the chatter. \
         Cite the thread ids behind each point as #{name}:a-b or #{name}:a.\n\
         At most {MAX_DIGEST_BYTES} bytes of plain text. Return only the digest, with no preface.\n\n\
         <thread>\n{}\n</thread>",
        lines.join("\n")
    )
}

fn cited_threads(text: &str) -> Vec<&str> {
    text.split('#')
        .skip(1)
        .filter_map(|rest| {
            let end = rest
                .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
                .unwrap_or(rest.len());
            rest[end..].starts_with(':').then(|| &rest[..end])
        })
        .filter(|prefix| !prefix.is_empty())
        .collect()
}

pub fn check_digest(reply: &str, name: &ThreadName) -> Result<String, String> {
    let digest = reply.trim();
    if digest.is_empty() {
        return Err("the digest is empty.".to_string());
    }
    if digest.len() > MAX_DIGEST_BYTES {
        return Err(format!(
            "the digest is {} bytes; the limit is {MAX_DIGEST_BYTES}.",
            digest.len()
        ));
    }
    let cited = cited_threads(digest);
    if let Some(other) = cited.iter().find(|prefix| **prefix != name.as_str()) {
        return Err(format!(
            "the digest cites #{other}: ids; cite only #{name}: ids."
        ));
    }
    if cited.is_empty() {
        return Err(format!("the digest cites no #{name}: ids."));
    }
    Ok(digest.to_string())
}

pub fn digest_with(
    prompt: &str,
    name: &ThreadName,
    curate: &mut impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut asked = prompt.to_string();
    let mut last = String::new();
    for _ in 0..MAX_MERGE_ATTEMPTS {
        match check_digest(&curate(&asked)?, name) {
            Ok(digest) => return Ok(digest),
            Err(e) => {
                asked = format!(
                    "{prompt}\n\nYour previous digest was rejected:\n{e}\n\nReturn a corrected digest only."
                );
                last = e;
            }
        }
    }
    Err(format!(
        "the digest was rejected {MAX_MERGE_ATTEMPTS} times; last: {last}"
    ))
}

pub fn merged_report(name: &ThreadName, digest: &str, at: &str) -> Report {
    Report {
        worker: format!("{THREAD_REPORT_PREFIX}{name}"),
        at: at.to_string(),
        status: MERGED_STATUS.to_string(),
        text: digest.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Own,
    Main,
    Thread(ThreadName),
}

pub fn split_scope(id: &str) -> Result<(Scope, &str), String> {
    let id = id.trim().trim_start_matches('#');
    match id.split_once(':') {
        None => Ok((Scope::Main, id)),
        Some((MAIN, rest)) => Ok((Scope::Main, rest)),
        Some((name, rest)) => Ok((Scope::Thread(ThreadName::parse(name)?), rest)),
    }
}

fn scope_name(s: &str) -> Result<Scope, String> {
    match s.trim() {
        MAIN => Ok(Scope::Main),
        s => ThreadName::parse(s).map(Scope::Thread),
    }
}

pub fn scope_arg(args: &Value) -> Result<Scope, String> {
    match args.get("thread") {
        None | Some(Value::Null) => Ok(Scope::Own),
        Some(Value::String(s)) => scope_name(s),
        Some(_) => Err("'thread' must be a string.".to_string()),
    }
}

pub fn in_scope(call: MemoryCall, scope: &str) -> Result<MemoryCall, String> {
    match call.tool {
        Tool::Recall(_) | Tool::Search(_) => Ok(MemoryCall {
            scope: scope_name(scope)?,
            tool: call.tool,
        }),
        Tool::Zoom(_) | Tool::Read(_) => Err(
            "--in only applies to recall and search; zoom and read take the thread in the id, as #name:3."
                .to_string(),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guarded {
    Drop,
    Merge,
    Park,
}

#[derive(Debug, Clone, Copy)]
pub struct Live<'a> {
    pub workers: &'a [String],
    pub records: usize,
}

pub fn guard_check(name: &ThreadName, verb: Guarded, live: Live) -> Result<(), String> {
    if !live.workers.is_empty() {
        return Err(format!(
            "thread {name} has running workers ({}); close them from thread {name}'s chat first.",
            live.workers.join(", ")
        ));
    }
    if verb == Guarded::Merge && live.records > 0 {
        let them = if live.records == 1 { "it" } else { "them" };
        return Err(format!(
            "thread {name} has {}; stop {them} from thread {name}'s chat before merging.",
            saved_records(live.records)
        ));
    }
    Ok(())
}

pub fn drop_message(name: &ThreadName, records: usize) -> String {
    match records {
        0 => format!("dropped thread {name}"),
        n => format!("dropped thread {name} and its {}", saved_records(n)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkCommand {
    Open(ThreadName),
    Usage(String),
}

pub fn parse_fork_args(args: &str) -> ForkCommand {
    let mut words = args.split_whitespace();
    match (words.next(), words.next()) {
        (Some(word), None) => match ThreadName::parse(word) {
            Ok(name) => ForkCommand::Open(name),
            Err(e) => ForkCommand::Usage(format!("{e}\n{FORK_USAGE}")),
        },
        _ => ForkCommand::Usage(FORK_USAGE.to_string()),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ForkSpawn<'a> {
    pub name: &'a ThreadName,
    pub exe: &'a str,
    pub main: &'a str,
    pub cwd: &'a str,
    pub block: Option<&'a str>,
    pub config_dir: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkLaunch {
    Rex(Vec<String>),
    Manual(String),
}

pub fn fork_launch(spawn: ForkSpawn) -> ForkLaunch {
    let config = spawn
        .config_dir
        .map(|dir| ["env".to_string(), format!("RALPH_CONFIG_DIR={dir}")]);
    let chat = [
        spawn.exe,
        "chat",
        "--thread",
        spawn.name.as_str(),
        "--store",
        spawn.main,
    ]
    .map(str::to_string);
    let command: Vec<String> = config.into_iter().flatten().chain(chat).collect();
    match spawn.block.filter(|block| !block.is_empty()) {
        Some(block) => {
            let label = format!("thread {}", spawn.name);
            let split = [
                "split", "-b", block, "--label", &label, "-c", spawn.cwd, "--",
            ];
            ForkLaunch::Rex(
                split
                    .map(str::to_string)
                    .into_iter()
                    .chain(command)
                    .collect(),
            )
        }
        None => {
            let quoted: Vec<String> = command.iter().map(|word| shell_quote(word)).collect();
            ForkLaunch::Manual(format!(
                "not in a Rex block, so /fork opens nothing; run this in another terminal:\n  {}",
                quoted.join(" ")
            ))
        }
    }
}

pub fn ls_counts(workers: usize, unread: usize) -> String {
    let mut counts = String::new();
    if workers > 0 {
        counts.push_str(&format!("  {}", counted(workers, "worker", "workers")));
    }
    if unread > 0 {
        counts.push_str(&format!("  {unread} unread"));
    }
    counts
}

fn saved_records(count: usize) -> String {
    counted(
        count,
        "saved watch or schedule",
        "saved watches or schedules",
    )
}

fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub fn label_item(item: &str, name: &ThreadName) -> String {
    for mark in ["#", "[#"] {
        if let Some(rest) = item.strip_prefix(mark) {
            return format!("{mark}{name}:{rest}");
        }
    }
    item.to_string()
}

pub fn label_lines(text: &str, name: &ThreadName) -> String {
    text.lines()
        .map(|line| label_item(line, name))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn fork_check(tool: &Tool, base: usize) -> Result<(), String> {
    let first_after = match tool {
        Tool::Zoom(node) if node.hi > base => node.lo.max(base),
        Tool::Read(request) if request.to >= base => request.from.max(base),
        _ => return Ok(()),
    };
    Err(format!(
        "#{first_after} is after this thread's fork point (#{base}); pull to see it."
    ))
}

pub fn thread_split(
    main_prefix: Vec<String>,
    base: usize,
    name: &ThreadName,
    own: ViewSplit,
) -> ViewSplit {
    let label = |lines: Vec<String>| -> Vec<String> {
        lines.iter().map(|line| label_item(line, name)).collect()
    };
    let mut older = vec![format!(
        "[main chat before this thread forked at #{base}; its ids have no prefix]"
    )];
    older.extend(main_prefix);
    older.push(format!("[thread {name}; its ids look like #{name}:0]"));
    older.extend(label(own.older));
    ViewSplit {
        older,
        middle: label(own.middle),
        tail: label(own.tail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::{Node, RangeRequest};
    use serde_json::json;

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    #[test]
    fn thread_names_are_short_lowercase_words() {
        assert_eq!(name("auth-fix2").as_str(), "auth-fix2");
        assert!(ThreadName::parse(&"a".repeat(32)).is_ok());
        for bad in [
            "",
            "-x",
            "Auth",
            "a b",
            "a/b",
            "a:b",
            "main",
            &"a".repeat(33),
        ] {
            assert!(ThreadName::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn thread_meta_round_trips_as_json() {
        let meta = ThreadMeta {
            base: 42,
            created: "2026-11-02T10:00:00Z".to_string(),
            merged: None,
            parked: None,
        };
        let text = serde_json::to_string(&meta).unwrap();
        assert_eq!(text, r#"{"base":42,"created":"2026-11-02T10:00:00Z"}"#);
        assert_eq!(serde_json::from_str::<ThreadMeta>(&text).unwrap(), meta);
        let merged = ThreadMeta {
            merged: Some(Merged {
                at: "2026-11-03T10:00:00Z".to_string(),
                digest: "Picked x. #auth:0".to_string(),
            }),
            ..meta.clone()
        };
        let text = serde_json::to_string(&merged).unwrap();
        assert!(
            text.ends_with(
                r#""merged":{"at":"2026-11-03T10:00:00Z","digest":"Picked x. #auth:0"}}"#
            ),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<ThreadMeta>(&text).unwrap(), merged);
        let parked = ThreadMeta {
            parked: Some(Parked {
                at: "2026-11-04T10:00:00Z".to_string(),
            }),
            ..meta
        };
        let text = serde_json::to_string(&parked).unwrap();
        assert_eq!(
            text,
            r#"{"base":42,"created":"2026-11-02T10:00:00Z","parked":{"at":"2026-11-04T10:00:00Z"}}"#
        );
        assert_eq!(serde_json::from_str::<ThreadMeta>(&text).unwrap(), parked);
    }

    fn merged_meta() -> ThreadMeta {
        ThreadMeta {
            base: 4,
            created: "c".to_string(),
            merged: Some(Merged {
                at: "a".to_string(),
                digest: "d".to_string(),
            }),
            parked: None,
        }
    }

    fn parked_meta() -> ThreadMeta {
        ThreadMeta {
            merged: None,
            parked: Some(Parked {
                at: "p".to_string(),
            }),
            ..merged_meta()
        }
    }

    #[test]
    fn a_parked_thread_does_not_open_until_revived() {
        let live = ThreadMeta {
            parked: None,
            ..parked_meta()
        };
        let auth = name("auth");
        assert!(open_check(&auth, &live).is_ok());
        assert_eq!(
            open_check(&auth, &parked_meta()).unwrap_err(),
            "thread auth is parked; `ralph thread revive auth` opens it again."
        );
        assert!(open_check(&auth, &merged_meta())
            .unwrap_err()
            .starts_with("thread auth is merged"));
        assert!(revive_check(&auth, &parked_meta()).is_ok());
        assert_eq!(
            revive_check(&auth, &live).unwrap_err(),
            "thread auth is not parked."
        );
        assert!(revive_check(&auth, &merged_meta())
            .unwrap_err()
            .starts_with("thread auth is merged"));
    }

    #[test]
    fn a_merged_thread_is_read_only() {
        let open = ThreadMeta {
            merged: None,
            ..merged_meta()
        };
        assert!(merged_check(&name("auth"), &open).is_ok());
        assert_eq!(
            merged_check(&name("auth"), &merged_meta()).unwrap_err(),
            "thread auth is merged into main and stays read-only; `ralph chat --thread auth history` still reads it."
        );
    }

    #[test]
    fn a_pull_says_how_far_the_fork_point_moved() {
        let auth = name("auth");
        assert_eq!(
            pull_message(&auth, 2, 5),
            "pulled thread auth: fork point #2 -> #5, 3 new main messages."
        );
        assert_eq!(
            pull_message(&auth, 4, 5),
            "pulled thread auth: fork point #4 -> #5, 1 new main message."
        );
        assert_eq!(
            pull_message(&auth, 5, 5),
            "thread auth is already at main's end (#5)."
        );
    }

    #[test]
    fn a_merged_thread_report_is_not_a_worker() {
        assert!(refuse_thread_report("GUZ-1").is_ok());
        assert!(refuse_thread_report("watch:w1").is_ok());
        assert_eq!(
            refuse_thread_report("thread:auth").unwrap_err(),
            "thread:auth is a merged thread's digest, not a worker; there is nothing to reply to, nudge or close."
        );
    }

    #[test]
    fn the_merge_prompt_carries_the_rules_and_the_thread_view() {
        let lines = vec![
            "#auth:0-1 picked a name".to_string(),
            "#auth:2 user x go".to_string(),
        ];
        let prompt = merge_prompt(&name("auth"), 64, &lines);
        for part in [
            "thread auth",
            "forked from main at #64",
            "#auth:a-b",
            "2048 bytes",
            "<thread>\n#auth:0-1 picked a name\n#auth:2 user x go\n</thread>",
        ] {
            assert!(prompt.contains(part), "{part}\n{prompt}");
        }
    }

    #[test]
    fn a_digest_is_short_and_cites_only_its_own_thread() {
        let auth = name("auth");
        assert_eq!(
            check_digest("  Picked x. #auth:0-1\n", &auth).unwrap(),
            "Picked x. #auth:0-1"
        );
        assert_eq!(
            check_digest(" \n", &auth).unwrap_err(),
            "the digest is empty."
        );
        let cite = " #auth:0";
        let fits = format!("{}{cite}", "x".repeat(MAX_DIGEST_BYTES - cite.len()));
        assert_eq!(check_digest(&fits, &auth).unwrap(), fits);
        assert_eq!(
            check_digest(&format!("x{fits}"), &auth).unwrap_err(),
            "the digest is 2049 bytes; the limit is 2048."
        );
        assert_eq!(
            check_digest("Picked x, see #5.", &auth).unwrap_err(),
            "the digest cites no #auth: ids."
        );
        assert_eq!(
            check_digest("Picked x #auth:0, as #b-side:3 said.", &auth).unwrap_err(),
            "the digest cites #b-side: ids; cite only #auth: ids."
        );
        assert!(check_digest("Kept main #5 in mind. #auth:2", &auth).is_ok());
    }

    #[test]
    fn a_digest_is_retried_then_given_up() {
        let auth = name("auth");
        let mut asked = Vec::new();
        let mut replies = vec!["Picked x. #auth:0", ""];
        let digest = digest_with("P", &auth, &mut |prompt: &str| {
            asked.push(prompt.to_string());
            Ok(replies.pop().unwrap().to_string())
        })
        .unwrap();
        assert_eq!(digest, "Picked x. #auth:0");
        assert_eq!(asked[0], "P");
        assert_eq!(
            asked[1],
            "P\n\nYour previous digest was rejected:\nthe digest is empty.\n\nReturn a corrected digest only."
        );
        let mut calls = 0;
        assert_eq!(
            digest_with("P", &auth, &mut |_: &str| {
                calls += 1;
                Ok("no cites".to_string())
            })
            .unwrap_err(),
            "the digest was rejected 3 times; last: the digest cites no #auth: ids."
        );
        assert_eq!(calls, MAX_MERGE_ATTEMPTS);
        assert_eq!(
            digest_with("P", &auth, &mut |_: &str| Err("exit 1".to_string())).unwrap_err(),
            "exit 1"
        );
    }

    #[test]
    fn a_merged_report_reads_as_a_thread_message() {
        let report = merged_report(&name("auth"), "Picked x. #auth:0", "2026-11-03T10:00:00Z");
        assert_eq!(report.worker, "thread:auth");
        assert_eq!(report.status, "merged");
        assert_eq!(report.at, "2026-11-03T10:00:00Z");
        let worker = Report {
            worker: "GUZ-1".to_string(),
            at: "t".to_string(),
            status: "done".to_string(),
            text: "ok".to_string(),
        };
        assert_eq!(
            crate::worker::chat_message(&[report, worker]),
            "[thread auth · merged]\nPicked x. #auth:0\n\n[worker GUZ-1 · done]\nok"
        );
    }

    #[test]
    fn ids_without_a_prefix_belong_to_main() {
        assert_eq!(split_scope("4-7").unwrap(), (Scope::Main, "4-7"));
        assert_eq!(split_scope("#5").unwrap(), (Scope::Main, "5"));
        assert_eq!(split_scope("main:5").unwrap(), (Scope::Main, "5"));
        assert_eq!(
            split_scope("#auth:0-3").unwrap(),
            (Scope::Thread(name("auth")), "0-3")
        );
        assert!(split_scope("Bad:1").is_err());
    }

    #[test]
    fn the_thread_argument_picks_the_search_space() {
        assert_eq!(scope_arg(&json!({})).unwrap(), Scope::Own);
        assert_eq!(scope_arg(&json!({"thread": "main"})).unwrap(), Scope::Main);
        assert_eq!(
            scope_arg(&json!({"thread": "auth"})).unwrap(),
            Scope::Thread(name("auth"))
        );
        assert!(scope_arg(&json!({"thread": 3})).is_err());
    }

    #[test]
    fn history_in_sets_the_scope_of_recall_and_search_only() {
        let call = |tool, args| crate::chat_memory::parse_tool_call(tool, &args).unwrap();
        let recall = in_scope(call("recall", json!({"pattern": "x"})), "main").unwrap();
        assert_eq!(recall.scope, Scope::Main);
        assert_eq!(recall.tool, Tool::Recall("x".to_string()));
        let search = in_scope(call("search", json!({"query": "x"})), "auth").unwrap();
        assert_eq!(search.scope, Scope::Thread(name("auth")));
        assert_eq!(
            in_scope(call("zoom", json!({"id": "3"})), "main").unwrap_err(),
            "--in only applies to recall and search; zoom and read take the thread in the id, as #name:3."
        );
        assert!(in_scope(call("recall", json!({"pattern": "x"})), "Bad").is_err());
    }

    #[test]
    fn workers_guard_every_verb_and_saved_records_guard_only_merge() {
        let auth = name("auth");
        let workers = ["w1".to_string(), "w2".to_string()];
        for verb in [Guarded::Drop, Guarded::Merge, Guarded::Park] {
            let idle = Live {
                workers: &[],
                records: 0,
            };
            assert!(guard_check(&auth, verb, idle).is_ok());
            let busy = Live {
                workers: &workers,
                records: 0,
            };
            assert_eq!(
                guard_check(&auth, verb, busy).unwrap_err(),
                "thread auth has running workers (w1, w2); close them from thread auth's chat first."
            );
        }
        let saved = |records| Live {
            workers: &[],
            records,
        };
        assert!(guard_check(&auth, Guarded::Drop, saved(3)).is_ok());
        assert!(guard_check(&auth, Guarded::Park, saved(3)).is_ok());
        assert_eq!(
            guard_check(&auth, Guarded::Merge, saved(1)).unwrap_err(),
            "thread auth has 1 saved watch or schedule; stop it from thread auth's chat before merging."
        );
        assert_eq!(
            guard_check(&auth, Guarded::Merge, saved(2)).unwrap_err(),
            "thread auth has 2 saved watches or schedules; stop them from thread auth's chat before merging."
        );
    }

    #[test]
    fn ls_counts_show_workers_and_unread_only_when_present() {
        assert_eq!(ls_counts(0, 0), "");
        assert_eq!(ls_counts(1, 0), "  1 worker");
        assert_eq!(ls_counts(0, 1), "  1 unread");
        assert_eq!(ls_counts(2, 3), "  2 workers  3 unread");
    }

    #[test]
    fn fork_args_take_one_thread_name() {
        assert_eq!(parse_fork_args(" auth "), ForkCommand::Open(name("auth")));
        assert_eq!(
            parse_fork_args(""),
            ForkCommand::Usage(FORK_USAGE.to_string())
        );
        assert_eq!(
            parse_fork_args("auth fix"),
            ForkCommand::Usage(FORK_USAGE.to_string())
        );
        for bad in ["Bad", "main"] {
            let ForkCommand::Usage(text) = parse_fork_args(bad) else {
                panic!("{bad} opened");
            };
            assert!(text.ends_with(FORK_USAGE), "{text}");
            assert!(text.len() > FORK_USAGE.len(), "{text}");
        }
    }

    fn spawn<'a>(auth: &'a ThreadName, block: Option<&'a str>) -> ForkSpawn<'a> {
        ForkSpawn {
            name: auth,
            exe: "/bin/ralph",
            main: "/s/chat",
            cwd: "/repo",
            block,
            config_dir: None,
        }
    }

    #[test]
    fn fork_splits_a_rex_block_running_the_thread_on_the_main_store() {
        let auth = name("auth");
        let words = |argv: &[&str]| -> Vec<String> { argv.iter().map(|w| w.to_string()).collect() };
        assert_eq!(
            fork_launch(spawn(&auth, Some("block:1"))),
            ForkLaunch::Rex(words(&[
                "split",
                "-b",
                "block:1",
                "--label",
                "thread auth",
                "-c",
                "/repo",
                "--",
                "/bin/ralph",
                "chat",
                "--thread",
                "auth",
                "--store",
                "/s/chat",
            ]))
        );
        let configured = ForkSpawn {
            config_dir: Some("/cfg"),
            ..spawn(&auth, Some("block:1"))
        };
        let ForkLaunch::Rex(argv) = fork_launch(configured) else {
            panic!("expected rex");
        };
        assert_eq!(
            argv[8..11],
            words(&["env", "RALPH_CONFIG_DIR=/cfg", "/bin/ralph"])
        );
    }

    #[test]
    fn fork_outside_rex_prints_the_command_to_run() {
        let auth = name("auth");
        for block in [None, Some("")] {
            assert_eq!(
                fork_launch(spawn(&auth, block)),
                ForkLaunch::Manual(
                    "not in a Rex block, so /fork opens nothing; run this in another terminal:\n  '/bin/ralph' 'chat' '--thread' 'auth' '--store' '/s/chat'"
                        .to_string()
                )
            );
        }
    }

    #[test]
    fn a_drop_says_how_many_saved_records_went_with_it() {
        let auth = name("auth");
        assert_eq!(drop_message(&auth, 0), "dropped thread auth");
        assert_eq!(
            drop_message(&auth, 1),
            "dropped thread auth and its 1 saved watch or schedule"
        );
        assert_eq!(
            drop_message(&auth, 2),
            "dropped thread auth and its 2 saved watches or schedules"
        );
    }

    #[test]
    fn labels_go_on_the_leading_id_only() {
        let auth = name("auth");
        assert_eq!(
            label_item("#4-7 talk about #2", &auth),
            "#auth:4-7 talk about #2"
        );
        assert_eq!(
            label_item("[#3: unreadable log line, skipped]", &auth),
            "[#auth:3: unreadable log line, skipped]"
        );
        assert_eq!(label_item("(end of range)", &auth), "(end of range)");
        assert_eq!(
            label_lines(
                "#1 · user · x\n#0 · user · y\n(zoom <id> for the full text)",
                &auth
            ),
            "#auth:1 · user · x\n#auth:0 · user · y\n(zoom <id> for the full text)"
        );
    }

    #[test]
    fn ids_at_or_after_the_fork_point_ask_for_a_pull() {
        let read = |from, to| {
            Tool::Read(RangeRequest {
                from,
                to,
                mode: crate::chat_memory::Mode::Raw,
                budget: 10,
                cursor: None,
            })
        };
        assert!(fork_check(&Tool::Zoom(Node::new(4, 8).unwrap()), 8).is_ok());
        assert!(fork_check(&read(0, 7), 8).is_ok());
        assert!(fork_check(&Tool::Recall("x".to_string()), 0).is_ok());
        assert_eq!(
            fork_check(&Tool::Zoom(Node::new(8, 9).unwrap()), 8).unwrap_err(),
            "#8 is after this thread's fork point (#8); pull to see it."
        );
        let first_after = "#8 is after this thread's fork point (#8); pull to see it.";
        assert_eq!(
            fork_check(&Tool::Zoom(Node::new(0, 16).unwrap()), 8).unwrap_err(),
            first_after
        );
        assert_eq!(fork_check(&read(5, 9), 8).unwrap_err(), first_after);
        assert_eq!(fork_check(&read(0, 8), 8).unwrap_err(), first_after);
    }

    #[test]
    fn a_thread_view_is_the_main_prefix_then_its_own_labelled_lines() {
        let own = ViewSplit {
            older: vec!["#0-1 started".to_string()],
            middle: vec!["#2 user x hi".to_string()],
            tail: vec!["#3 assistant x yo".to_string()],
        };
        let split = thread_split(vec!["#0-63 old work".to_string()], 64, &name("auth"), own);
        assert_eq!(
            split.older,
            vec![
                "[main chat before this thread forked at #64; its ids have no prefix]",
                "#0-63 old work",
                "[thread auth; its ids look like #auth:0]",
                "#auth:0-1 started",
            ]
        );
        assert_eq!(split.middle, vec!["#auth:2 user x hi"]);
        assert_eq!(split.tail, vec!["#auth:3 assistant x yo"]);
    }
}
