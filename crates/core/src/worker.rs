use crate::chat_memory::{Entry, Role};
use crate::mcp::{dispatch, string_arg, tool_result, Dispatch};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const SERVER_NAME: &str = "ralph-worker";
pub const INBOX_DIR: &str = "inbox";
pub const WORKERS_DIR: &str = "workers";
pub const WORKTREE_DIR: &str = ".worktrees";

pub const WORKER_PROMPT: &str = "\
You work for one person on the task you were given, in your own git worktree. Act; do not stop to ask for confirmation.

When you finish, get blocked, or need a decision, call the report tool, then stop and wait. The person's answer arrives as your next message. Never send progress updates.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub worker: String,
    pub at: String,
    pub status: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Silent,
    Respond(Value),
    Report {
        id: Value,
        status: String,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Spawn { id: String, task: String },
    Reply { id: String, text: String },
}

pub fn command_definitions() -> [Value; 2] {
    [
        json!({
            "name": "spawn_worker",
            "description": "Start a worker agent on a task in its own git worktree and tmux session. Only when the person asked for it in their latest message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Short id such as a ticket key: letters, digits and dashes."},
                    "task": {"type": "string", "description": "What the worker should do, in the person's terms."}
                },
                "required": ["id", "task"]
            }
        }),
        json!({
            "name": "reply",
            "description": "Send a message to a running worker. Only with what the person decided in their latest message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The worker id."},
                    "text": {"type": "string", "description": "The message for the worker."}
                },
                "required": ["id", "text"]
            }
        }),
    ]
}

pub fn parse_command(name: &str, args: &Value) -> Option<Result<Command, String>> {
    let parsed = match name {
        "spawn_worker" => string_arg(args, "task")
            .and_then(|task| worker_id(args).map(|id| Command::Spawn { id, task })),
        "reply" => string_arg(args, "text")
            .and_then(|text| worker_id(args).map(|id| Command::Reply { id, text })),
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

pub fn person_has_floor(entries: &[Entry]) -> Result<(), String> {
    match entries.last() {
        Some(entry) if entry.role == Role::User => Ok(()),
        _ => Err(
            "Only the person can direct workers. Relay this to them and wait for their answer."
                .to_string(),
        ),
    }
}

pub fn session_name(id: &str) -> String {
    format!("ralph-{id}")
}

pub fn branch_name(id: &str) -> String {
    format!("worker/{id}")
}

pub fn worktree_path(repo: &Path, id: &str) -> PathBuf {
    repo.join(WORKTREE_DIR).join(id)
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

/// Runs Claude through a login, interactive zsh so the worker gets the same
/// environment as a terminal the person opens; `$0 "$@"` keeps the arguments unquoted.
pub fn worker_command(mcp_config: &Path, task: &str) -> Vec<String> {
    [
        "zsh",
        "-lic",
        r#"exec "$0" "$@""#,
        "claude",
        "--mcp-config",
        &mcp_config.to_string_lossy(),
        "--append-system-prompt",
        WORKER_PROMPT,
        task,
    ]
    .map(str::to_string)
    .to_vec()
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

pub fn startup_state(pane: &str) -> Startup {
    if pane.contains("Yes, I trust this folder") {
        Startup::TrustPrompt
    } else if pane.contains("Claude Code v") {
        Startup::Ready
    } else {
        Startup::Starting
    }
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "report",
            "description": "Send a message to the person who dispatched you. Use it only when you finish, get blocked, or need a decision, then stop and wait for their answer. Each report costs a full chat turn, so never send progress updates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": {"type": "string", "description": "One word: done, blocked, or question."},
                    "text": {"type": "string", "description": "What happened, or what you need decided."}
                },
                "required": ["status", "text"]
            }
        }
    ])
}

fn parse_report(name: &str, args: &Value) -> Result<(String, String), String> {
    match name {
        "report" => Ok((string_arg(args, "status")?, string_arg(args, "text")?)),
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

    fn entry(role: Role) -> Entry {
        Entry {
            role,
            at: "t".into(),
            text: "x".into(),
        }
    }

    #[test]
    fn parse_command_reads_spawn_and_reply() {
        assert_eq!(
            parse_command("spawn_worker", &json!({"id": "GUZ-1", "task": "fix it"})),
            Some(Ok(Command::Spawn {
                id: "GUZ-1".into(),
                task: "fix it".into()
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
    fn parse_command_rejects_ids_unsafe_for_paths_and_tmux() {
        for id in ["../x", "a b", "a:b", "a/b"] {
            assert!(matches!(
                parse_command("reply", &json!({"id": id, "text": "t"})),
                Some(Err(_))
            ));
        }
    }

    #[test]
    fn person_has_floor_only_after_a_user_message() {
        assert!(person_has_floor(&[entry(Role::Assistant), entry(Role::User)]).is_ok());
        assert!(person_has_floor(&[entry(Role::User), entry(Role::Worker)]).is_err());
        assert!(person_has_floor(&[]).is_err());
    }

    #[test]
    fn worker_command_runs_claude_through_zsh_and_ends_with_the_task() {
        let args = worker_command(Path::new("/s/workers/A.json"), "do it");
        assert_eq!(args[..2], ["zsh", "-lic"]);
        assert_eq!(args[3..6], ["claude", "--mcp-config", "/s/workers/A.json"]);
        assert_eq!(args.last().unwrap(), "do it");
    }

    #[test]
    fn startup_state_reads_the_pane() {
        assert_eq!(startup_state(""), Startup::Starting);
        assert_eq!(
            startup_state(" Accessing workspace:\n ❯ No, exit\n   Yes, I trust this folder\n"),
            Startup::TrustPrompt
        );
        assert_eq!(
            startup_state(" ▐▛███▛█   Claude Code v2.1.295\n❯ fix it\n"),
            Startup::Ready
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
                status: "done".into(),
                text: "hello".into()
            }
        );
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
