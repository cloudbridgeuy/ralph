use crate::chat_memory::Trigger;
use crate::mcp::{dispatch, string_arg, tool_result, Dispatch};
use crate::sanitize::strip_controls;
use crate::watch_request::WatchRequest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

mod acp;
mod headless;
mod hooks;
mod kind;
mod markers;
mod mcp;
mod opencode_http;
mod ownership;
mod rollback;
mod worktree;
pub use acp::*;
pub use headless::*;
pub use hooks::*;
pub use kind::*;
pub use markers::*;
pub use mcp::*;
pub use opencode_http::*;
pub use ownership::*;
pub use rollback::*;
pub use worktree::*;

pub const SERVER_NAME: &str = "ralph-worker";
pub const INBOX_DIR: &str = "inbox";
pub const WORKERS_DIR: &str = "workers";
pub const WORKTREE_DIR: &str = ".worktrees";
pub const WORKTREE_DIR_ENV: &str = "RALPH_WORKTREE_DIR";
pub const OPENCODE_CONFIG_FILE: &str = "opencode.json";
pub const READY_DIR: &str = "ready";
pub const REPORTED_DIR: &str = "reported";
pub const BLOCKED_DIR: &str = "blocked";
pub const WAITING_DIR: &str = "waiting";
pub const ATTACHMENTS_DIR: &str = "attachments";
pub const UNREPORTED_DIR: &str = "unreported";
pub const SENT_DIR: &str = "sent";
pub const ACTIVITY_DIR: &str = "activity";
pub const GONE_DIR: &str = "gone";
pub const DEFAULT_OPENCODE_MODEL: &str = "openai/gpt-6.1-sol";
pub const OPENCODE_HARNESS: &str = "opencode";
pub const STORE_OPTION: &str = "@ralph_store";
pub const ID_OPTION: &str = "@ralph_id";
pub const SESSION_FORMAT: &str = "#{session_name}\t#{@ralph_store}\t#{@ralph_id}";

pub fn is_model_id(model: &str) -> bool {
    matches!(model.split_once('/'), Some((provider, name)) if !provider.is_empty() && !name.is_empty())
}

pub fn resolve_opencode_model(
    spawn: Option<&str>,
    config: Result<Option<&str>, &str>,
) -> (String, Option<String>) {
    if let Some(model) = spawn {
        return (model.to_string(), None);
    }
    match config {
        Err(error) => (
            DEFAULT_OPENCODE_MODEL.to_string(),
            Some(format!(
                "Warning: [opencode] model was skipped because the ralph config did not load ({error}). Used \"{DEFAULT_OPENCODE_MODEL}\" instead."
            )),
        ),
        Ok(None) => (DEFAULT_OPENCODE_MODEL.to_string(), None),
        Ok(Some(model)) if is_model_id(model) => (model.to_string(), None),
        Ok(Some(model)) => (
            DEFAULT_OPENCODE_MODEL.to_string(),
            Some(format!(
                "[opencode] model '{model}' must look like provider/model; using \"{DEFAULT_OPENCODE_MODEL}\"."
            )),
        ),
    }
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Spawn {
        id: String,
        task: String,
        harness: HarnessId,
        model: Option<String>,
    },
    Reply {
        id: String,
        text: String,
        interrupt: bool,
    },
    Close {
        id: String,
        abandon: bool,
        force: bool,
    },
    Watch(WatchRequest),
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
        Trigger::Private => "private",
    }
}

pub fn parse_pending_turn(marker: &str) -> Option<Trigger> {
    match marker.trim() {
        "user" => Some(Trigger::User),
        "worker" => Some(Trigger::Worker),
        "private" => Some(Trigger::Private),
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

pub fn worker_mcp_args(id: &str, inbox: &Path, worktree: Option<&Path>) -> Vec<String> {
    let mut args = vec![
        "worker-mcp".to_string(),
        "--id".to_string(),
        id.to_string(),
        "--inbox".to_string(),
        inbox.to_string_lossy().into_owned(),
    ];
    if let Some(worktree) = worktree {
        args.push("--worktree".to_string());
        args.push(worktree.to_string_lossy().into_owned());
    }
    args
}

pub fn worker_mcp_config(exe: &Path, id: &str, inbox: &Path, worktree: Option<&Path>) -> Value {
    json!({
        "mcpServers": {
            SERVER_NAME: {
                "command": exe.to_string_lossy(),
                "args": worker_mcp_args(id, inbox, worktree)
            }
        }
    })
}

pub fn opencode_config(model: &str, exe: &Path, mcp_args: &[String], attachments: &Path) -> Value {
    let command: Vec<String> = std::iter::once(exe.to_string_lossy().into_owned())
        .chain(mcp_args.iter().cloned())
        .collect();
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
                "command": command,
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

pub fn exclude_lines(harness: HarnessId, repo: &Path, worktree_dir: &Path) -> Vec<String> {
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
    if harness == HarnessId::Opencode {
        lines.push(format!("/{OPENCODE_CONFIG_FILE}"));
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
            strip_controls(&string_arg(args, "text")?).into_owned(),
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

    #[test]
    fn opencode_model_resolution_prefers_spawn_then_config_then_default() {
        assert_eq!(
            resolve_opencode_model(Some("a/b"), Ok(Some("c/d"))),
            ("a/b".to_string(), None)
        );
        assert_eq!(
            resolve_opencode_model(None, Ok(Some("c/d"))),
            ("c/d".to_string(), None)
        );
        assert_eq!(
            resolve_opencode_model(None, Ok(None)),
            (DEFAULT_OPENCODE_MODEL.to_string(), None)
        );
    }

    #[test]
    fn opencode_model_resolution_warns_on_invalid_config() {
        for bad in ["", "sol", "openai/", "/sol"] {
            let (model, warning) = resolve_opencode_model(None, Ok(Some(bad)));
            assert_eq!(model, DEFAULT_OPENCODE_MODEL);
            assert!(warning.unwrap().contains("[opencode] model"), "{bad}");
        }
        assert_eq!(
            resolve_opencode_model(Some("a/b"), Ok(Some(""))),
            ("a/b".to_string(), None)
        );
    }

    #[test]
    fn opencode_model_resolution_warns_when_config_fails_to_load() {
        let (model, warning) = resolve_opencode_model(None, Err("bad toml at line 3"));
        assert_eq!(model, DEFAULT_OPENCODE_MODEL);
        let warning = warning.unwrap();
        assert!(warning.contains("bad toml at line 3"));
        assert!(warning.contains(DEFAULT_OPENCODE_MODEL));
    }

    #[test]
    fn opencode_model_resolution_ignores_config_error_when_spawn_model_given() {
        assert_eq!(
            resolve_opencode_model(Some("a/b"), Err("boom")),
            ("a/b".to_string(), None)
        );
    }

    #[test]
    fn model_id_needs_provider_and_name() {
        assert!(is_model_id("openai/gpt-6.1-sol"));
        for bad in ["", "sol", "openai/", "/sol"] {
            assert!(!is_model_id(bad), "{bad}");
        }
    }

    fn report(worker: &str, status: &str, text: &str) -> Report {
        Report {
            worker: worker.into(),
            at: "2026-10-09T15:30:01.5+00:00".into(),
            status: status.into(),
            text: text.into(),
        }
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
        assert!(command_allowed(&close, Some(Trigger::Private)).is_err());
        assert!(command_allowed(&close, None).is_err());
    }

    #[test]
    fn spawn_allowed_only_in_a_user_turn() {
        let spawn = Command::Spawn {
            id: "A".into(),
            task: "t".into(),
            harness: HarnessId::Claude,
            model: None,
        };
        assert!(command_allowed(&spawn, Some(Trigger::User)).is_ok());
        assert!(command_allowed(&spawn, Some(Trigger::Worker)).is_err());
        assert!(command_allowed(&spawn, None).is_err());
    }

    #[test]
    fn watch_allowed_only_in_a_user_turn() {
        let args = json!({"id": "ci", "command": "c", "follow_up": "f"});
        let Some(Ok(watch)) = parse_command("watch", &args) else {
            panic!("expected a watch command")
        };
        assert!(command_allowed(&watch, Some(Trigger::User)).is_ok());
        assert!(command_allowed(&watch, Some(Trigger::Worker)).is_err());
        assert!(command_allowed(&watch, None).is_err());
    }

    #[test]
    fn reply_allowed_in_a_user_or_worker_turn() {
        let reply = Command::Reply {
            id: "A".into(),
            text: "t".into(),
            interrupt: false,
        };
        assert!(command_allowed(&reply, Some(Trigger::User)).is_ok());
        assert!(command_allowed(&reply, Some(Trigger::Worker)).is_ok());
        assert!(command_allowed(&reply, Some(Trigger::Private)).is_err());
        assert!(command_allowed(&reply, None).is_err());
    }

    #[test]
    fn pending_turn_marker_round_trips() {
        for trigger in [Trigger::User, Trigger::Worker, Trigger::Private] {
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
    fn worker_command_output_is_exactly_the_tmux_launch_vector() {
        let args = worker_command(
            Path::new("/s/workers/A.json"),
            Path::new("/s/workers/A.settings.json"),
            "auto",
            Path::new("/s/attachments"),
            "do it",
        );
        assert_eq!(
            args,
            vec![
                "zsh".to_string(),
                "-lic".to_string(),
                r#"exec "$0" "$@""#.to_string(),
                "claude".to_string(),
                "--permission-mode".to_string(),
                "auto".to_string(),
                "--mcp-config".to_string(),
                "/s/workers/A.json".to_string(),
                "--settings".to_string(),
                "/s/workers/A.settings.json".to_string(),
                "--add-dir".to_string(),
                "/s/attachments".to_string(),
                "--append-system-prompt".to_string(),
                WORKER_PROMPT.to_string(),
                "do it".to_string(),
            ]
        );
    }

    #[test]
    fn opencode_config_sets_model_and_worker_server() {
        let config = opencode_config(
            "openai/gpt-6.1-sol",
            Path::new("/bin/ralph"),
            &worker_mcp_args("A", Path::new("/s/inbox"), Some(Path::new("/w"))),
            Path::new("/s/attachments"),
        );
        assert_eq!(config["model"], "openai/gpt-6.1-sol");
        assert_eq!(config["permission"]["question"], "deny");
        assert_eq!(
            config["permission"]["external_directory"],
            json!({ "/s/attachments/**": "allow" })
        );
        assert_eq!(
            config["mcp"][SERVER_NAME],
            json!({
                "type": "local",
                "command": [
                    "/bin/ralph", "worker-mcp", "--id", "A", "--inbox", "/s/inbox",
                    "--worktree", "/w"
                ],
                "enabled": true
            })
        );
    }

    #[test]
    fn exclude_lines_add_opencode_config_only_for_opencode() {
        let repo = Path::new("/repo");
        let dir = repo.join(WORKTREE_DIR);
        assert_eq!(
            exclude_lines(HarnessId::Claude, repo, &dir),
            ["/.worktrees/"]
        );
        assert_eq!(
            exclude_lines(HarnessId::Opencode, repo, &dir),
            ["/.worktrees/", "/opencode.json"]
        );
    }

    #[test]
    fn exclude_lines_cover_only_a_worktree_dir_inside_the_repo() {
        let repo = Path::new("/repo");
        assert_eq!(
            exclude_lines(HarnessId::Claude, repo, Path::new("/repo/tmp/wt")),
            ["/tmp/wt/"]
        );
        assert!(exclude_lines(HarnessId::Claude, repo, Path::new("/elsewhere/wt")).is_empty());
        assert!(exclude_lines(HarnessId::Claude, repo, Path::new("/repo/../wt")).is_empty());
        assert!(exclude_lines(HarnessId::Claude, repo, repo).is_empty());
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
        let config = worker_mcp_config(
            Path::new("/bin/ralph"),
            "A",
            Path::new("/s/inbox"),
            Some(Path::new("/w")),
        );
        assert_eq!(
            config["mcpServers"][SERVER_NAME]["args"],
            json!([
                "worker-mcp",
                "--id",
                "A",
                "--inbox",
                "/s/inbox",
                "--worktree",
                "/w"
            ])
        );
    }

    #[test]
    fn worker_mcp_config_omits_the_worktree_without_one() {
        let config = worker_mcp_config(Path::new("/bin/ralph"), "A", Path::new("/s/inbox"), None);
        assert_eq!(
            config["mcpServers"][SERVER_NAME]["args"],
            json!(["worker-mcp", "--id", "A", "--inbox", "/s/inbox"])
        );
    }

    #[test]
    fn report_text_strips_nul_and_other_controls() {
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "report", "arguments": {
            "status": "done",
            "text": "a\0b\u{0001}c\n\td`e\u{007f}"
        }}});
        assert_eq!(
            handle_request(&request),
            Outcome::Report {
                id: json!(3),
                status: ReportStatus::Done,
                text: "abc\n\td`e".into()
            }
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
