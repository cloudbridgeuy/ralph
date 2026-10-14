use crate::chat_memory::Trigger;
use crate::mcp::{dispatch, string_arg, tool_result, Dispatch};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

mod hooks;
mod markers;
mod ownership;
mod worktree;
pub use hooks::*;
pub use markers::*;
pub use ownership::*;
pub use worktree::*;

pub const SERVER_NAME: &str = "ralph-worker";
pub const INBOX_DIR: &str = "inbox";
pub const WORKERS_DIR: &str = "workers";
pub const WORKTREE_DIR: &str = ".worktrees";
pub const WORKTREE_DIR_ENV: &str = "RALPH_WORKTREE_DIR";
pub const OPENCODE_CONFIG_FILE: &str = "opencode.json";
pub const OPENCODE_PLUGIN_FILE: &str = ".opencode/plugins/ralph-worker.ts";
pub const READY_DIR: &str = "ready";
pub const REPORTED_DIR: &str = "reported";
pub const BLOCKED_DIR: &str = "blocked";
pub const WAITING_DIR: &str = "waiting";
pub const ATTACHMENTS_DIR: &str = "attachments";
pub const UNREPORTED_DIR: &str = "unreported";
pub const DEFAULT_OPENCODE_MODEL: &str = "openai/gpt-5.6-sol";
pub const OPENCODE_HARNESS: &str = "opencode";
pub const STORE_OPTION: &str = "@ralph_store";
pub const HARNESS_OPTION: &str = "@ralph_harness";
pub const ID_OPTION: &str = "@ralph_id";
pub const SESSION_FORMAT: &str =
    "#{session_name}\t#{@ralph_store}\t#{@ralph_harness}\t#{@ralph_id}";

pub const WORKER_PROMPT: &str = "\
You work for one person on the task you were given, in your own git worktree. Act, don't ask: make reasonable decisions yourself and keep going until the task is done. Never ask for confirmation or permission, and never ask questions in your reply text or with a built-in question tool; nobody reads them.

End every turn with the report tool: status done when the task is finished, blocked when you cannot go on, or question when you need a decision only the person can make, and a full summary in text. Then stop and wait. The person's answer arrives as your next message. Never send progress updates.

A reference like [Image #N: <path>] in a message is an image file: read it at that path.";

pub const REPORT_REMINDER: &str = "You stopped without calling report. Call report now with status done, blocked or question and a full summary of what you did or what you need.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub worker: String,
    pub at: String,
    pub status: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportStatus {
    Done,
    Blocked,
    Question,
}

impl ReportStatus {
    pub const ALL: [ReportStatus; 3] = [Self::Done, Self::Blocked, Self::Question];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Question => "question",
        }
    }

    fn parse(status: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|known| known.as_str() == status)
            .ok_or_else(|| format!("Status '{status}' must be done, blocked or question."))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Silent,
    Respond(Value),
    Report {
        id: Value,
        status: ReportStatus,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Opencode { model: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Spawn {
        id: String,
        task: String,
        harness: Harness,
    },
    Reply {
        id: String,
        text: String,
    },
    Close {
        id: String,
        abandon: bool,
        force: bool,
    },
}

pub fn command_definitions() -> [Value; 3] {
    [
        json!({
            "name": "spawn_worker",
            "description": "Start a worker agent on a task in its own git worktree and tmux session. Only when the person asked for it in their latest message. When the worker needs a pasted image, copy its [Image #N: <path>] reference verbatim into task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Short id such as a ticket key: letters, digits and dashes."},
                    "task": {"type": "string", "description": "What the worker should do, in the person's terms. Keep [Image #N: <path>] references verbatim."},
                    "harness": {"type": "string", "enum": ["claude", "opencode"], "description": "Agent to run. Default claude."},
                    "model": {"type": "string", "description": "opencode only: provider/model. Default openai/gpt-5.6-sol."}
                },
                "required": ["id", "task"]
            }
        }),
        json!({
            "name": "reply",
            "description": "Send a message to a running worker. Only with what the person asked in their latest message, or, when a worker asks, with an answer the person already settled in the chat.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The worker id."},
                    "text": {"type": "string", "description": "The message for the worker. Keep [Image #N: <path>] references verbatim when it needs the image."}
                },
                "required": ["id", "text"]
            }
        }),
        json!({
            "name": "close_worker",
            "description": "Close a worker of this chat: stop its tmux session and opencode session, remove its worktree and delete its worker/<id> branch. Refuses while the branch has commits not in main or the worktree has uncommitted changes, unless abandon is true, and refuses a worker of another chat unless force is true. Only when the person asked for it in their latest message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The worker id."},
                    "abandon": {"type": "boolean", "description": "Drop unmerged commits and uncommitted changes. Only when the person said so. Default false."},
                    "force": {"type": "boolean", "description": "Close even when the session is tagged with another chat's store or is untagged. Only when the person said so. Default false."}
                },
                "required": ["id"]
            }
        }),
    ]
}

pub fn parse_command(name: &str, args: &Value) -> Option<Result<Command, String>> {
    let parsed = match name {
        "spawn_worker" => string_arg(args, "task").and_then(|task| {
            let id = worker_id(args)?;
            let harness = harness(args)?;
            Ok(Command::Spawn { id, task, harness })
        }),
        "reply" => string_arg(args, "text")
            .and_then(|text| worker_id(args).map(|id| Command::Reply { id, text })),
        "close_worker" => worker_id(args).and_then(|id| {
            let flag = |name: &str| match args.get(name) {
                None | Some(Value::Null) => Ok(false),
                Some(Value::Bool(value)) => Ok(*value),
                Some(_) => Err(format!("{name} must be true or false.")),
            };
            Ok(Command::Close {
                id,
                abandon: flag("abandon")?,
                force: flag("force")?,
            })
        }),
        _ => return None,
    };
    Some(parsed)
}

fn worker_id(args: &Value) -> Result<String, String> {
    let id = string_arg(args, "id")?;
    if id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(id)
    } else {
        Err(format!(
            "Worker id '{id}' may only use letters, digits and dashes."
        ))
    }
}

fn harness(args: &Value) -> Result<Harness, String> {
    let model = args.get("model").and_then(Value::as_str);
    match (args.get("harness").and_then(Value::as_str), model) {
        (None | Some("claude"), None) => Ok(Harness::Claude),
        (None | Some("claude"), Some(_)) => {
            Err("A model can only be set for the opencode harness.".to_string())
        }
        (Some(OPENCODE_HARNESS), model) => {
            let model = model.unwrap_or(DEFAULT_OPENCODE_MODEL);
            match model.split_once('/') {
                Some((provider, name)) if !provider.is_empty() && !name.is_empty() => {
                    Ok(Harness::Opencode {
                        model: model.to_string(),
                    })
                }
                _ => Err(format!("Model '{model}' must look like provider/model.")),
            }
        }
        (Some(other), _) => Err(format!(
            "Unknown harness '{other}'. Use claude or opencode."
        )),
    }
}

/// Workers are directed only in a turn the person started; a worker-triggered turn may only reply.
pub fn command_allowed(command: &Command, pending: Option<Trigger>) -> Result<(), String> {
    match (command, pending) {
        (_, Some(Trigger::User)) | (Command::Reply { .. }, Some(Trigger::Worker)) => Ok(()),
        _ => Err(
            "Only the person can direct workers. Relay this to them and wait for their answer."
                .to_string(),
        ),
    }
}

pub const PENDING_TURN_FILE: &str = "turn";

pub fn pending_turn_marker(trigger: Trigger) -> &'static str {
    match trigger {
        Trigger::User => "user",
        Trigger::Worker => "worker",
    }
}

pub fn parse_pending_turn(marker: &str) -> Option<Trigger> {
    match marker.trim() {
        "user" => Some(Trigger::User),
        "worker" => Some(Trigger::Worker),
        _ => None,
    }
}

pub fn branch_name(id: &str) -> String {
    format!("worker/{id}")
}

pub fn worktree_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(id)
}

/// Arguments for `git worktree add`: reuse the worker branch if an earlier spawn created it.
pub fn worktree_add_args(path: &Path, branch: &str, branch_exists: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["worktree".into(), "add".into()];
    if branch_exists {
        args.extend([path.into(), branch.into()]);
    } else {
        args.extend(["-b".into(), branch.into(), path.into()]);
    }
    args
}

pub fn wt_switch_args(
    repo: &Path,
    path: &Path,
    branch: &str,
    branch_exists: bool,
) -> Vec<OsString> {
    let pinned_path = Value::from(path.to_string_lossy()).to_string();
    let mut args: Vec<OsString> = vec![
        "-C".into(),
        repo.into(),
        "--config-set".into(),
        format!("worktree-path={pinned_path}").into(),
        "switch".into(),
    ];
    if !branch_exists {
        args.extend(["--create".into(), "--base".into(), "@".into()]);
    }
    args.extend([branch.into(), "--no-cd".into()]);
    args
}

#[derive(Debug, PartialEq)]
pub enum WorktreeVia {
    Existing,
    Wt,
    Git { wt_error: Option<String> },
}

impl WorktreeVia {
    pub fn note(&self) -> String {
        match self {
            Self::Existing => "Reused its existing worktree.".to_string(),
            Self::Wt => "Created its worktree with wt.".to_string(),
            Self::Git { wt_error: None } => {
                "Created its worktree with git (wt is not installed).".to_string()
            }
            Self::Git {
                wt_error: Some(error),
            } => format!("Created its worktree with git because wt failed: {error}"),
        }
    }
}

pub fn worker_mcp_config(exe: &Path, id: &str, inbox: &Path) -> Value {
    json!({
        "mcpServers": {
            SERVER_NAME: {
                "command": exe.to_string_lossy(),
                "args": ["worker-mcp", "--id", id, "--inbox", inbox.to_string_lossy()]
            }
        }
    })
}

pub fn opencode_config(
    model: &str,
    exe: &Path,
    id: &str,
    inbox: &Path,
    attachments: &Path,
) -> Value {
    json!({
        "$schema": "https://opencode.ai/config.json",
        "model": model,
        "permission": {
            "question": "deny",
            "external_directory": { format!("{}/**", attachments.to_string_lossy()): "allow" }
        },
        "mcp": {
            SERVER_NAME: {
                "type": "local",
                "command": [exe.to_string_lossy(), "worker-mcp", "--id", id, "--inbox", inbox.to_string_lossy()],
                "enabled": true
            }
        }
    })
}

pub fn worker_command(
    mcp_config: &Path,
    settings: &Path,
    permission_mode: &str,
    attachments: &Path,
    task: &str,
) -> Vec<String> {
    [
        "zsh",
        "-lic",
        r#"exec "$0" "$@""#,
        "claude",
        "--permission-mode",
        permission_mode,
        "--mcp-config",
        &mcp_config.to_string_lossy(),
        "--settings",
        &settings.to_string_lossy(),
        "--add-dir",
        &attachments.to_string_lossy(),
        "--append-system-prompt",
        WORKER_PROMPT,
        task,
    ]
    .map(str::to_string)
    .to_vec()
}

pub fn opencode_session_id(id: &str) -> String {
    format!("ses_ralph{id}")
}

pub fn opencode_command(id: &str, task: &str) -> Vec<String> {
    let session = opencode_session_id(id);
    let prompt = format!("{WORKER_PROMPT}\n\n{task}");
    [
        "zsh",
        "-lic",
        r#"exec "$0" "$@""#,
        "opencode",
        "--auto",
        "--session",
        &session,
        "--prompt",
        &prompt,
    ]
    .map(str::to_string)
    .to_vec()
}

pub fn exclude_lines(harness: &Harness, repo: &Path, worktree_dir: &Path) -> Vec<String> {
    let mut lines: Vec<String> = worktree_dir
        .strip_prefix(repo)
        .ok()
        .filter(|inside| {
            inside.components().next().is_some()
                && inside
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
        })
        .map(|inside| format!("/{}/", inside.to_string_lossy()))
        .into_iter()
        .collect();
    if let Harness::Opencode { .. } = harness {
        lines.push(format!("/{OPENCODE_CONFIG_FILE}"));
        lines.push(format!("/{OPENCODE_PLUGIN_FILE}"));
    }
    lines
}

/// Claude asks before working in a folder it has not seen; "No, exit" is preselected,
/// so accepting takes Down then Enter.
pub const TRUST_KEYS: [&str; 2] = ["Down", "Enter"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Startup {
    Starting,
    TrustPrompt,
    Ready,
}

pub fn startup_state(pane: &str, ready: bool) -> Startup {
    if ready {
        Startup::Ready
    } else if pane.contains("Yes, I trust this folder") {
        Startup::TrustPrompt
    } else {
        Startup::Starting
    }
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "report",
            "description": "Send a message to the person who dispatched you. Call it at the end of every turn: when you finish, get blocked, or need a decision. Then stop and wait for their answer. Each report costs a full chat turn, so never send progress updates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "enum": ReportStatus::ALL.map(ReportStatus::as_str), "description": "done: task finished. blocked: you cannot go on. question: you need a decision."},
                    "text": {"type": "string", "description": "Full summary: what you did and what you found, or what blocks you, or what you need decided."}
                },
                "required": ["status", "text"]
            }
        }
    ])
}

fn parse_report(name: &str, args: &Value) -> Result<(ReportStatus, String), String> {
    match name {
        "report" => Ok((
            ReportStatus::parse(&string_arg(args, "status")?)?,
            string_arg(args, "text")?,
        )),
        other => Err(format!("Unknown tool '{other}'.")),
    }
}

pub fn handle_request(request: &Value) -> Outcome {
    match dispatch(SERVER_NAME, &tool_definitions(), request) {
        Dispatch::Silent => Outcome::Silent,
        Dispatch::Respond(v) => Outcome::Respond(v),
        Dispatch::Call { id, name, args } => match parse_report(&name, &args) {
            Ok((status, text)) => Outcome::Report { id, status, text },
            Err(message) => Outcome::Respond(tool_result(id, &message, true)),
        },
    }
}

pub fn inbox_file_name(report: &Report, seq: u64) -> String {
    let stamp: String = report.at.chars().filter(char::is_ascii_digit).collect();
    let worker: String = report
        .worker
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{stamp}-{seq:06}-{worker}.json")
}

pub fn chat_message(reports: &[Report]) -> String {
    reports
        .iter()
        .map(|r| format!("[worker {} · {}]\n{}", r.worker, r.status, r.text))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(worker: &str, status: &str, text: &str) -> Report {
        Report {
            worker: worker.into(),
            at: "2026-10-09T15:30:01.5+00:00".into(),
            status: status.into(),
            text: text.into(),
        }
    }

    #[test]
    fn parse_command_reads_spawn_and_reply() {
        assert_eq!(
            parse_command("spawn_worker", &json!({"id": "GUZ-1", "task": "fix it"})),
            Some(Ok(Command::Spawn {
                id: "GUZ-1".into(),
                task: "fix it".into(),
                harness: Harness::Claude
            }))
        );
        assert_eq!(
            parse_command("reply", &json!({"id": "GUZ-1", "text": "merge"})),
            Some(Ok(Command::Reply {
                id: "GUZ-1".into(),
                text: "merge".into()
            }))
        );
        assert_eq!(parse_command("zoom", &json!({})), None);
    }

    #[test]
    fn parse_command_reads_close_with_optional_abandon() {
        assert_eq!(
            parse_command("close_worker", &json!({"id": "GUZ-1"})),
            Some(Ok(Command::Close {
                id: "GUZ-1".into(),
                abandon: false,
                force: false
            }))
        );
        assert_eq!(
            parse_command("close_worker", &json!({"id": "GUZ-1", "abandon": true})),
            Some(Ok(Command::Close {
                id: "GUZ-1".into(),
                abandon: true,
                force: false
            }))
        );
        assert_eq!(
            parse_command("close_worker", &json!({"id": "GUZ-1", "force": true})),
            Some(Ok(Command::Close {
                id: "GUZ-1".into(),
                abandon: false,
                force: true
            }))
        );
        assert!(matches!(
            parse_command("close_worker", &json!({"id": "GUZ-1", "force": 1})),
            Some(Err(_))
        ));
        assert!(matches!(
            parse_command("close_worker", &json!({"id": "GUZ-1", "abandon": "yes"})),
            Some(Err(_))
        ));
        assert!(matches!(
            parse_command("close_worker", &json!({"id": "../x"})),
            Some(Err(_))
        ));
    }

    #[test]
    fn close_allowed_only_in_a_user_turn() {
        let close = Command::Close {
            id: "A".into(),
            abandon: false,
            force: false,
        };
        assert!(command_allowed(&close, Some(Trigger::User)).is_ok());
        assert!(command_allowed(&close, Some(Trigger::Worker)).is_err());
        assert!(command_allowed(&close, None).is_err());
    }

    fn spawn_harness(args: Value) -> Result<Harness, String> {
        let mut args = args;
        args["id"] = json!("A");
        args["task"] = json!("t");
        match parse_command("spawn_worker", &args) {
            Some(Ok(Command::Spawn { harness, .. })) => Ok(harness),
            Some(Ok(other)) => panic!("unexpected {other:?}"),
            Some(Err(e)) => Err(e),
            None => panic!("not a worker command"),
        }
    }

    #[test]
    fn spawn_harness_defaults_to_claude_and_opencode_to_sol() {
        assert_eq!(spawn_harness(json!({})), Ok(Harness::Claude));
        assert_eq!(
            spawn_harness(json!({"harness": "claude"})),
            Ok(Harness::Claude)
        );
        assert_eq!(
            spawn_harness(json!({"harness": "opencode"})),
            Ok(Harness::Opencode {
                model: DEFAULT_OPENCODE_MODEL.into()
            })
        );
        assert_eq!(
            spawn_harness(json!({"harness": "opencode", "model": "openai/gpt-5.6-luna"})),
            Ok(Harness::Opencode {
                model: "openai/gpt-5.6-luna".into()
            })
        );
    }

    #[test]
    fn spawn_harness_rejects_bad_choices() {
        for args in [
            json!({"harness": "codex"}),
            json!({"model": "openai/gpt-5.6-sol"}),
            json!({"harness": "opencode", "model": "gpt-5.6-sol"}),
            json!({"harness": "opencode", "model": "openai/"}),
            json!({"harness": "opencode", "model": "/sol"}),
        ] {
            assert!(spawn_harness(args.clone()).is_err(), "{args}");
        }
    }

    #[test]
    fn parse_command_rejects_ids_unsafe_for_paths_and_tmux() {
        for id in ["../x", "a b", "a:b", "a/b"] {
            assert!(matches!(
                parse_command("reply", &json!({"id": id, "text": "t"})),
                Some(Err(_))
            ));
        }
    }

    #[test]
    fn spawn_allowed_only_in_a_user_turn() {
        let spawn = Command::Spawn {
            id: "A".into(),
            task: "t".into(),
            harness: Harness::Claude,
        };
        assert!(command_allowed(&spawn, Some(Trigger::User)).is_ok());
        assert!(command_allowed(&spawn, Some(Trigger::Worker)).is_err());
        assert!(command_allowed(&spawn, None).is_err());
    }

    #[test]
    fn reply_allowed_in_a_user_or_worker_turn() {
        let reply = Command::Reply {
            id: "A".into(),
            text: "t".into(),
        };
        assert!(command_allowed(&reply, Some(Trigger::User)).is_ok());
        assert!(command_allowed(&reply, Some(Trigger::Worker)).is_ok());
        assert!(command_allowed(&reply, None).is_err());
    }

    #[test]
    fn pending_turn_marker_round_trips() {
        for trigger in [Trigger::User, Trigger::Worker] {
            assert_eq!(
                parse_pending_turn(pending_turn_marker(trigger)),
                Some(trigger)
            );
        }
        assert_eq!(parse_pending_turn("junk"), None);
        assert_eq!(parse_pending_turn(""), None);
    }

    #[test]
    fn worker_command_runs_claude_through_zsh_and_ends_with_the_task() {
        let args = worker_command(
            Path::new("/s/workers/A.json"),
            Path::new("/s/workers/A.settings.json"),
            "auto",
            Path::new("/s/attachments"),
            "do it",
        );
        assert_eq!(args[..2], ["zsh", "-lic"]);
        assert_eq!(
            args[3..12],
            [
                "claude",
                "--permission-mode",
                "auto",
                "--mcp-config",
                "/s/workers/A.json",
                "--settings",
                "/s/workers/A.settings.json",
                "--add-dir",
                "/s/attachments"
            ]
        );
        assert_eq!(args.last().unwrap(), "do it");
    }

    #[test]
    fn opencode_command_runs_the_tui_in_the_worker_session_with_rules_and_task() {
        let args = opencode_command("GUZ-1", "do it");
        assert_eq!(args[..2], ["zsh", "-lic"]);
        assert_eq!(
            args[3..8],
            [
                "opencode",
                "--auto",
                "--session",
                "ses_ralphGUZ-1",
                "--prompt"
            ]
        );
        let prompt = args.last().unwrap();
        assert!(prompt.starts_with(WORKER_PROMPT));
        assert!(prompt.ends_with("\n\ndo it"));
    }

    #[test]
    fn opencode_config_sets_model_and_worker_server() {
        let config = opencode_config(
            "openai/gpt-5.6-sol",
            Path::new("/bin/ralph"),
            "A",
            Path::new("/s/inbox"),
            Path::new("/s/attachments"),
        );
        assert_eq!(config["model"], "openai/gpt-5.6-sol");
        assert_eq!(config["permission"]["question"], "deny");
        assert_eq!(
            config["permission"]["external_directory"],
            json!({ "/s/attachments/**": "allow" })
        );
        assert_eq!(
            config["mcp"][SERVER_NAME],
            json!({
                "type": "local",
                "command": ["/bin/ralph", "worker-mcp", "--id", "A", "--inbox", "/s/inbox"],
                "enabled": true
            })
        );
    }

    #[test]
    fn exclude_lines_add_opencode_config_only_for_opencode() {
        let repo = Path::new("/repo");
        let dir = repo.join(WORKTREE_DIR);
        assert_eq!(
            exclude_lines(&Harness::Claude, repo, &dir),
            ["/.worktrees/"]
        );
        assert_eq!(
            exclude_lines(
                &Harness::Opencode {
                    model: "a/b".into()
                },
                repo,
                &dir
            ),
            [
                "/.worktrees/",
                "/opencode.json",
                "/.opencode/plugins/ralph-worker.ts"
            ]
        );
    }

    #[test]
    fn exclude_lines_cover_only_a_worktree_dir_inside_the_repo() {
        let repo = Path::new("/repo");
        assert_eq!(
            exclude_lines(&Harness::Claude, repo, Path::new("/repo/tmp/wt")),
            ["/tmp/wt/"]
        );
        assert!(exclude_lines(&Harness::Claude, repo, Path::new("/elsewhere/wt")).is_empty());
        assert!(exclude_lines(&Harness::Claude, repo, Path::new("/repo/../wt")).is_empty());
        assert!(exclude_lines(&Harness::Claude, repo, repo).is_empty());
    }

    #[test]
    fn worktree_add_args_reuses_an_existing_branch() {
        let path = Path::new("/repo/.worktrees/GUZ-1");
        assert_eq!(
            worktree_add_args(path, "worker/GUZ-1", false),
            [
                "worktree",
                "add",
                "-b",
                "worker/GUZ-1",
                "/repo/.worktrees/GUZ-1"
            ]
        );
        assert_eq!(
            worktree_add_args(path, "worker/GUZ-1", true),
            ["worktree", "add", "/repo/.worktrees/GUZ-1", "worker/GUZ-1"]
        );
    }

    #[test]
    fn wt_switch_args_pin_the_worktree_path_and_create_only_a_new_branch() {
        let repo = Path::new("/repo");
        let path = Path::new("/repo/.worktrees/GUZ-1");
        assert_eq!(
            wt_switch_args(repo, path, "worker/GUZ-1", false),
            [
                "-C",
                "/repo",
                "--config-set",
                "worktree-path=\"/repo/.worktrees/GUZ-1\"",
                "switch",
                "--create",
                "--base",
                "@",
                "worker/GUZ-1",
                "--no-cd"
            ]
        );
        assert_eq!(
            wt_switch_args(repo, path, "worker/GUZ-1", true),
            [
                "-C",
                "/repo",
                "--config-set",
                "worktree-path=\"/repo/.worktrees/GUZ-1\"",
                "switch",
                "worker/GUZ-1",
                "--no-cd"
            ]
        );
    }

    #[test]
    fn wt_switch_args_escape_quotes_in_the_path() {
        let args = wt_switch_args(Path::new("/r"), Path::new("/r/a\"b"), "worker/A", true);
        assert_eq!(args[3], "worktree-path=\"/r/a\\\"b\"");
    }

    #[test]
    fn worktree_via_note_names_the_path_used() {
        assert_eq!(WorktreeVia::Wt.note(), "Created its worktree with wt.");
        assert_eq!(
            WorktreeVia::Git { wt_error: None }.note(),
            "Created its worktree with git (wt is not installed)."
        );
        assert_eq!(
            WorktreeVia::Git {
                wt_error: Some("boom".into())
            }
            .note(),
            "Created its worktree with git because wt failed: boom"
        );
        assert_eq!(
            WorktreeVia::Existing.note(),
            "Reused its existing worktree."
        );
    }

    #[test]
    fn startup_state_waits_for_the_ready_signal_and_spots_the_trust_prompt() {
        let trust = " Accessing workspace:\n ❯ No, exit\n   Yes, I trust this folder\n";
        assert_eq!(startup_state("", false), Startup::Starting);
        assert_eq!(startup_state(trust, false), Startup::TrustPrompt);
        assert_eq!(startup_state(trust, true), Startup::Ready);
        assert_eq!(
            startup_state(" ▐▛███▛█   Claude Code v2.1.295\n", false),
            Startup::Starting
        );
        assert_eq!(
            startup_state("  ctrl+p commands\n", false),
            Startup::Starting
        );
    }

    #[test]
    fn worker_mcp_config_points_at_inbox() {
        let config = worker_mcp_config(Path::new("/bin/ralph"), "A", Path::new("/s/inbox"));
        assert_eq!(
            config["mcpServers"][SERVER_NAME]["args"],
            json!(["worker-mcp", "--id", "A", "--inbox", "/s/inbox"])
        );
    }

    #[test]
    fn report_call_yields_status_and_text() {
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "report", "arguments": {"status": "done", "text": "hello"}}});
        assert_eq!(
            handle_request(&request),
            Outcome::Report {
                id: json!(3),
                status: ReportStatus::Done,
                text: "hello".into()
            }
        );
    }

    #[test]
    fn report_schema_and_parser_accept_only_known_statuses() {
        assert_eq!(
            tool_definitions()[0]["inputSchema"]["properties"]["status"]["enum"],
            json!(["done", "blocked", "question"])
        );
        for status in ReportStatus::ALL {
            assert_eq!(ReportStatus::parse(status.as_str()), Ok(status));
        }
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "report", "arguments": {"status": "ok", "text": "hello"}}});
        let Outcome::Respond(v) = handle_request(&request) else {
            panic!("expected error response");
        };
        assert_eq!(v["result"]["isError"], true);
    }

    #[test]
    fn report_without_text_is_tool_error() {
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "report", "arguments": {"status": "done"}}});
        let Outcome::Respond(v) = handle_request(&request) else {
            panic!("expected error response");
        };
        assert_eq!(v["result"]["isError"], true);
    }

    #[test]
    fn inbox_file_name_sorts_by_time_and_sanitizes_worker() {
        assert_eq!(
            inbox_file_name(&report("GUZ/1 2", "done", "x"), 7),
            "2026100915300150000-000007-GUZ_1_2.json"
        );
    }

    #[test]
    fn chat_message_joins_reports_with_headers() {
        let reports = [report("A", "done", "one"), report("B", "blocked", "two")];
        assert_eq!(
            chat_message(&reports),
            "[worker A · done]\none\n\n[worker B · blocked]\ntwo"
        );
    }
}
