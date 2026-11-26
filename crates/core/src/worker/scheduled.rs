use super::acp::REPORT_TOOL;
use super::{
    agy_auto_allow_option, claude_args, control_response, disallowed_tool_args, mcptools_decision,
    permission_response, BlockedContent, ClaudeArgs, ClaudePermission, McptoolsDecision,
    PermissionMode, PermissionOption, Report, SandboxMode, WorkerId, ALLOW_ONCE, SERVER_NAME,
    WORKER_PROMPT,
};
use crate::chat::{MCPTOOLS_ASK_RULES, MCPTOOLS_SERVER};
use serde_json::{json, Value};
use std::ffi::OsString;

pub const UNATTENDED_REFUSAL: &str = "the request was refused because this worker runs unattended";

pub const SCHEDULED_WORKER_PROMPT: &str = "\
You work for one person on the task you were given, in your own git worktree. The task was scheduled and nobody is watching it run. Act, don't ask: make reasonable decisions yourself and keep going until the task is done. Never ask for confirmation or permission, and never ask questions in your reply text or with a built-in question tool; nobody reads them.

Never push, merge, open a pull request, or change Linear. Commit only on your own branch.

Finish with one report tool call: status done when the task is finished, or status blocked with the reason when you cannot go on and only the person can unblock you. End the report text with an `assumptions:` line naming every decision you made where the task was ambiguous. Write `assumptions: none` when the task was clear. Then stop. Never wait for an answer and never send progress updates.

Call set_status with one sentence about what you are doing when you start and whenever your current activity changes; it is not a report and costs no turn. If you are waiting on your own subagents, checks or builds, call set_status and keep waiting in this turn; that is not blocked.

A reference like [Image #N: <path>] in a message is an image file: read it at that path.";

pub fn worker_prompt(scheduled: bool) -> &'static str {
    if scheduled {
        SCHEDULED_WORKER_PROMPT
    } else {
        WORKER_PROMPT
    }
}

pub const SCHEDULED_PERMISSION_MODE: &str = "auto";

const WORKER_TOOLS: [&str; 2] = [REPORT_TOOL, "set_status"];

pub fn scheduled_disallowed_args(copied: &[String]) -> Vec<String> {
    let mut args = disallowed_tool_args();
    if copied.iter().any(|name| name == MCPTOOLS_SERVER) {
        args.extend(MCPTOOLS_ASK_RULES.map(str::to_string));
    }
    args
}

pub fn headless_claude_args(options: &ClaudeArgs, scheduled: bool) -> Vec<String> {
    if !scheduled {
        return claude_args(options);
    }
    let mut args = claude_args(&ClaudeArgs {
        mode: PermissionMode::Yolo,
        ..*options
    });
    args.extend(scheduled_disallowed_args(options.copied));
    args
}

pub fn scheduled_sandbox_mode(
    scheduled: bool,
    configured: Option<SandboxMode>,
) -> Option<SandboxMode> {
    match (scheduled, configured) {
        (false, _) | (true, Some(SandboxMode::ReadOnly)) => configured,
        (true, _) => Some(SandboxMode::WorkspaceWrite),
    }
}

pub fn scheduled_permission_mode(scheduled: bool, configured: PermissionMode) -> PermissionMode {
    if scheduled {
        PermissionMode::Ask
    } else {
        configured
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unattended<'a> {
    Allow(&'a PermissionOption),
    Deny(&'a PermissionOption),
    Cancel,
}

fn is_worker_tool(content: &BlockedContent) -> bool {
    let tool = content.tool.trim();
    tool == SERVER_NAME
        || WORKER_TOOLS
            .iter()
            .any(|name| tool == format!("{SERVER_NAME}-{name}: {name}"))
}

fn option_of_kind<'a>(content: &'a BlockedContent, kind: &str) -> Option<&'a PermissionOption> {
    content.options.iter().find(|option| option.kind == kind)
}

fn refusal(content: &BlockedContent) -> Unattended<'_> {
    option_of_kind(content, "reject_once").map_or(Unattended::Cancel, Unattended::Deny)
}

pub fn unattended(content: &BlockedContent) -> Unattended<'_> {
    is_worker_tool(content)
        .then(|| option_of_kind(content, ALLOW_ONCE))
        .flatten()
        .map_or_else(|| refusal(content), Unattended::Allow)
}

pub fn unattended_acp<'a>(
    agy: bool,
    request: &Value,
    content: &'a BlockedContent,
) -> Unattended<'a> {
    if !agy {
        return unattended(content);
    }
    agy_auto_allow_option(request, content).map_or_else(|| refusal(content), Unattended::Allow)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnattendedAnswer {
    pub line: String,
    pub report: Option<Report>,
}

pub fn unattended_claude_answer(
    worker: &WorkerId,
    at: &str,
    ask: &ClaudePermission,
) -> UnattendedAnswer {
    let id = &ask.content.request_id;
    match mcptools_decision(ask) {
        McptoolsDecision::Read => UnattendedAnswer {
            line: permission_response(id, ALLOW_ONCE, &ask.input),
            report: None,
        },
        _ => UnattendedAnswer {
            line: unattended_claude_deny(id),
            report: Some(denied_report(worker, at, &ask.content)),
        },
    }
}

fn unattended_claude_deny(request_id: &str) -> String {
    let message = format!(
        "ralph denied this tool call: {UNATTENDED_REFUSAL}. Do not retry it; say in your report what you needed it for."
    );
    control_response(request_id, &json!({"behavior": "deny", "message": message}))
}

impl<'a> Unattended<'a> {
    pub fn option_id(&self) -> Option<&'a str> {
        match self {
            Self::Allow(option) | Self::Deny(option) => Some(&option.option_id),
            Self::Cancel => None,
        }
    }

    pub fn denial_report(
        &self,
        worker: &WorkerId,
        at: &str,
        content: &BlockedContent,
    ) -> Option<Report> {
        match self {
            Self::Allow(_) => None,
            Self::Deny(_) | Self::Cancel => Some(denied_report(worker, at, content)),
        }
    }
}

fn denied_report(worker: &WorkerId, at: &str, content: &BlockedContent) -> Report {
    Report::ralph_blocked(
        worker,
        at,
        format!(
            "Scheduled worker {} was denied a permission request on {}: {}. The task was scheduled and nobody was watching, so nobody was asked.",
            worker.as_str(),
            content.tool, content.summary
        ),
    )
}

pub const ORIGIN_MAIN: &str = "origin/main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeBase {
    Head,
    OriginMain,
}

impl WorktreeBase {
    pub fn scheduled(scheduled: bool) -> Self {
        if scheduled {
            Self::OriginMain
        } else {
            Self::Head
        }
    }
}

pub fn fetch_origin_args() -> Vec<OsString> {
    vec!["fetch".into(), "origin".into()]
}

pub const FETCH_TIMEOUT_SECS: u64 = 60;

pub const FETCH_GRACE_SECS: u64 = 2;

const DEFAULT_SSH: &str = "ssh";
const BATCH_MODE_OPTION: &str = "-o BatchMode=yes";

fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

pub fn fetch_ssh_command(env: Option<&str>, config: Option<&str>) -> String {
    let base = present(env).or(present(config)).unwrap_or(DEFAULT_SSH);
    format!("{base} {BATCH_MODE_OPTION}")
}

pub fn fetch_env(ssh_command: &str) -> [(&'static str, String); 2] {
    [
        ("GIT_TERMINAL_PROMPT", "0".to_string()),
        ("GIT_SSH_COMMAND", ssh_command.to_string()),
    ]
}

pub fn fetch_timeout_text() -> String {
    format!("git fetch origin timed out after {FETCH_TIMEOUT_SECS}s")
}

#[cfg(test)]
mod tests {
    use super::super::{
        worker_command, worktree_add_args, wt_switch_args, ClaudeSession, ClaudeShell,
        ClaudeWorkerArgs,
    };
    use super::*;
    use crate::worker::{ReportKind, ReportStatus};
    use std::path::Path;

    fn claude_args() -> ClaudeWorkerArgs<'static> {
        ClaudeWorkerArgs {
            store: Path::new("/s"),
            mcp_config: Path::new("/s/workers/A.json"),
            settings: Path::new("/s/workers/A.settings.json"),
            permission_mode: "auto",
            attachments: Path::new("/s/attachments"),
            task_dir: Path::new("/s/workers/A.task"),
            model: None,
            scheduled: false,
            shell: ClaudeShell::Zsh,
            config_dir: None,
            copied: &[],
        }
    }

    #[test]
    fn an_unset_config_dir_keeps_the_worker_argv() {
        assert_eq!(
            worker_command(&claude_args()),
            [
                "zsh",
                "-lic",
                r#"exec "$0" "$@""#,
                "env",
                "RALPH_INBOX=/s/inbox",
                "claude",
                "--permission-mode",
                "auto",
                "--mcp-config",
                "/s/workers/A.json",
                "--strict-mcp-config",
                "--allowedTools",
                "mcp__ralph-worker__report,mcp__ralph-worker__set_status",
                "--settings",
                "/s/workers/A.settings.json",
                "--add-dir",
                "/s/attachments",
                "--add-dir",
                "/s/workers/A.task",
                "--append-system-prompt",
                worker_prompt(false),
                "Read your task from /s/workers/A.task/task.md",
            ]
        );
    }

    #[test]
    fn a_config_dir_puts_env_before_claude() {
        let args = worker_command(&ClaudeWorkerArgs {
            config_dir: Some(Path::new("/c/alt dir")),
            ..claude_args()
        });
        assert_eq!(
            args[..7],
            [
                "zsh",
                "-lic",
                r#"exec "$0" "$@""#,
                "env",
                "RALPH_INBOX=/s/inbox",
                "CLAUDE_CONFIG_DIR=/c/alt dir",
                "claude"
            ]
        );
    }

    #[test]
    fn scheduled_worktree_add_branches_from_origin_main() {
        let path = Path::new("/repo/.worktrees/sched-n");
        assert_eq!(
            worktree_add_args(path, "worker/sched-n", false, WorktreeBase::OriginMain),
            [
                "worktree",
                "add",
                "-b",
                "worker/sched-n",
                "/repo/.worktrees/sched-n",
                "origin/main"
            ]
        );
        assert_eq!(
            worktree_add_args(path, "worker/sched-n", true, WorktreeBase::OriginMain),
            [
                "worktree",
                "add",
                "/repo/.worktrees/sched-n",
                "worker/sched-n"
            ]
        );
    }

    #[test]
    fn scheduled_wt_switch_bases_a_new_branch_on_origin_main() {
        let args = wt_switch_args(
            Path::new("/repo"),
            Path::new("/repo/.worktrees/sched-n"),
            "worker/sched-n",
            false,
            WorktreeBase::OriginMain,
        );
        assert_eq!(
            args[5..9],
            ["--create", "--base", "origin/main", "worker/sched-n"]
        );
    }

    #[test]
    fn the_base_follows_the_scheduled_flag() {
        assert_eq!(WorktreeBase::scheduled(false), WorktreeBase::Head);
        assert_eq!(WorktreeBase::scheduled(true), WorktreeBase::OriginMain);
    }

    #[test]
    fn fetch_origin_args_fetch_only_origin() {
        assert_eq!(fetch_origin_args(), ["fetch", "origin"]);
    }

    #[test]
    fn the_flag_selects_the_prompt() {
        assert_eq!(worker_prompt(false), WORKER_PROMPT);
        assert_eq!(worker_prompt(true), SCHEDULED_WORKER_PROMPT);
        assert!(SCHEDULED_WORKER_PROMPT.contains("Never push, merge, open a pull request"));
        assert!(SCHEDULED_WORKER_PROMPT.contains("Never wait for an answer"));
        assert!(!SCHEDULED_WORKER_PROMPT.contains("wait for it"));
        assert!(!SCHEDULED_WORKER_PROMPT.contains("status question"));
    }

    #[test]
    fn both_prompts_ask_for_an_assumptions_line() {
        for prompt in [WORKER_PROMPT, SCHEDULED_WORKER_PROMPT] {
            assert!(prompt.contains("assumptions:"), "{prompt}");
            assert!(prompt.contains("assumptions: none"), "{prompt}");
        }
    }

    #[test]
    fn a_scheduled_worker_command_blocks_push_tools_before_a_flag() {
        let args = worker_command(&ClaudeWorkerArgs {
            scheduled: true,
            shell: ClaudeShell::Zsh,
            ..claude_args()
        });
        let at = args.iter().position(|a| a == "--disallowedTools").unwrap();
        assert_eq!(
            args[at..at + 5],
            [
                "--disallowedTools",
                "Bash(git push:*)",
                "Bash(gh pr:*)",
                "Bash(gh api:*)",
                "--append-system-prompt"
            ]
        );
        assert_eq!(
            args.last().unwrap(),
            "Read your task from /s/workers/A.task/task.md"
        );
    }

    fn mode_after_flag(args: &[String]) -> &str {
        let at = args.iter().position(|a| a == "--permission-mode").unwrap();
        &args[at + 1]
    }

    #[test]
    fn a_scheduled_worker_forces_auto_mode_whatever_the_chat_uses() {
        let chat = ClaudeWorkerArgs {
            permission_mode: "bypassPermissions",
            ..claude_args()
        };
        let scheduled = ClaudeWorkerArgs {
            scheduled: true,
            shell: ClaudeShell::Zsh,
            ..chat
        };
        assert_eq!(mode_after_flag(&worker_command(&scheduled)), "auto");
    }

    #[test]
    fn the_fetch_never_prompts_and_has_a_timeout_text() {
        assert_eq!(
            fetch_env("ssh -o BatchMode=yes"),
            [
                ("GIT_TERMINAL_PROMPT", "0".to_string()),
                ("GIT_SSH_COMMAND", "ssh -o BatchMode=yes".to_string())
            ]
        );
        assert_eq!(fetch_timeout_text(), "git fetch origin timed out after 60s");
    }

    #[test]
    fn the_ssh_command_prefers_env_then_config_then_plain_ssh() {
        assert_eq!(
            fetch_ssh_command(Some("ssh -i /k"), Some("ssh -F /c")),
            "ssh -i /k -o BatchMode=yes"
        );
        assert_eq!(
            fetch_ssh_command(None, Some("ssh -F /c")),
            "ssh -F /c -o BatchMode=yes"
        );
        assert_eq!(fetch_ssh_command(None, None), "ssh -o BatchMode=yes");
    }

    #[test]
    fn blank_ssh_values_count_as_absent() {
        assert_eq!(
            fetch_ssh_command(Some(""), Some("ssh -F /c")),
            "ssh -F /c -o BatchMode=yes"
        );
        assert_eq!(
            fetch_ssh_command(Some("  "), Some(" \t")),
            "ssh -o BatchMode=yes"
        );
        assert_eq!(
            fetch_ssh_command(None, Some("  ssh -F /c \n")),
            "ssh -F /c -o BatchMode=yes"
        );
    }

    #[test]
    fn the_repo_ssh_key_survives_in_the_fetch_command() {
        assert_eq!(
            fetch_ssh_command(
                None,
                Some("ssh -i ~/.ssh/id_cloudbridge_admin -F /dev/null")
            ),
            "ssh -i ~/.ssh/id_cloudbridge_admin -F /dev/null -o BatchMode=yes"
        );
    }

    #[test]
    fn an_unscheduled_worker_keeps_the_chat_mode() {
        let chat = ClaudeWorkerArgs {
            permission_mode: "acceptEdits",
            ..claude_args()
        };
        assert_eq!(mode_after_flag(&worker_command(&chat)), "acceptEdits");
    }

    #[test]
    fn an_unscheduled_worker_command_blocks_nothing() {
        assert!(!worker_command(&claude_args())
            .iter()
            .any(|a| a == "--disallowedTools"));
    }

    #[test]
    fn a_scheduled_worker_command_carries_the_scheduled_prompt() {
        let args = worker_command(&ClaudeWorkerArgs {
            scheduled: true,
            shell: ClaudeShell::Zsh,
            ..claude_args()
        });
        let at = args
            .iter()
            .position(|a| a == "--append-system-prompt")
            .unwrap();
        assert_eq!(args[at + 1], SCHEDULED_WORKER_PROMPT);
    }

    #[test]
    fn scheduled_prompt_says_waiting_on_subagents_is_not_blocked() {
        assert!(
            SCHEDULED_WORKER_PROMPT.contains("only the person can unblock you"),
            "{SCHEDULED_WORKER_PROMPT}"
        );
        assert!(
            SCHEDULED_WORKER_PROMPT.contains("subagents, checks or builds")
                && SCHEDULED_WORKER_PROMPT
                    .contains("call set_status and keep waiting in this turn")
                && SCHEDULED_WORKER_PROMPT.contains("not blocked"),
            "{SCHEDULED_WORKER_PROMPT}"
        );
    }

    fn option(id: &str, kind: &str) -> PermissionOption {
        PermissionOption {
            option_id: id.to_string(),
            name: id.to_string(),
            kind: kind.to_string(),
        }
    }

    fn content(tool: &str, options: Vec<PermissionOption>) -> BlockedContent {
        BlockedContent {
            request_id: "r1".to_string(),
            tool: tool.to_string(),
            summary: "do it".to_string(),
            options,
            pending: Vec::new(),
        }
    }

    fn codex_options() -> Vec<PermissionOption> {
        vec![
            option("accept", "allow_once"),
            option("decline", "reject_once"),
            option("cancel", "reject_once"),
        ]
    }

    fn claude_ask(tool: &str, input: serde_json::Value) -> ClaudePermission {
        ClaudePermission {
            input,
            content: content(tool, Vec::new()),
        }
    }

    #[test]
    fn scheduled_disallowed_args_add_the_mcptools_pair_only_when_it_is_copied() {
        let bare = scheduled_disallowed_args(&[]);
        assert_eq!(bare, disallowed_tool_args());
        let copied = scheduled_disallowed_args(&[MCPTOOLS_SERVER.to_string()]);
        assert_eq!(copied[..bare.len()], bare[..]);
        assert_eq!(
            copied[bare.len()..],
            ["mcp__mcptools__execute", "mcp__mcptools__call_tool"]
        );
    }

    fn headless(mode: PermissionMode, scheduled: bool, copied: &[String]) -> Vec<String> {
        let options = ClaudeArgs {
            model: None,
            mcp_config: "/c.json",
            attachments: Path::new("/a"),
            mode,
            session: ClaudeSession::New("sid"),
            copied,
        };
        headless_claude_args(&options, scheduled)
    }

    #[test]
    fn a_scheduled_claude_headless_worker_runs_in_auto_mode_with_the_deny_list() {
        let copied = [MCPTOOLS_SERVER.to_string()];
        for mode in [PermissionMode::Ask, PermissionMode::Yolo] {
            let args = headless(mode, true, &copied);
            assert_eq!(mode_after_flag(&args), "auto");
            let deny = scheduled_disallowed_args(&copied);
            assert_eq!(args[args.len() - deny.len()..], deny[..]);
        }
    }

    #[test]
    fn an_unscheduled_claude_headless_worker_keeps_its_mode_and_has_no_deny_list() {
        let args = headless(PermissionMode::Ask, false, &[]);
        assert_eq!(mode_after_flag(&args), "default");
        assert!(!args.iter().any(|arg| arg == "--disallowedTools"));
    }

    #[test]
    fn unattended_denies_with_the_reject_option_and_never_asks() {
        let codex = content("command", codex_options());
        assert_eq!(unattended(&codex), Unattended::Deny(&codex.options[1]));
        let elicitation = content(
            "mcp server",
            vec![
                option("decline", "reject_once"),
                option("cancel", "reject_once"),
            ],
        );
        assert_eq!(
            unattended(&elicitation),
            Unattended::Deny(&elicitation.options[0])
        );
        let opencode = content(
            "bash",
            vec![
                option("once", "allow_once"),
                option("always", "allow_always"),
                option("reject", "reject_once"),
            ],
        );
        assert_eq!(
            unattended(&opencode),
            Unattended::Deny(&opencode.options[2])
        );
    }

    #[test]
    fn unattended_cancels_when_no_option_rejects() {
        let only_allow = content("bash", vec![option("allow", "allow_once")]);
        assert_eq!(unattended(&only_allow), Unattended::Cancel);
        assert_eq!(unattended(&content("bash", Vec::new())), Unattended::Cancel);
    }

    #[test]
    fn a_scheduled_worker_always_asks_whatever_is_configured() {
        for configured in [PermissionMode::Ask, PermissionMode::Yolo] {
            assert_eq!(
                scheduled_permission_mode(true, configured),
                PermissionMode::Ask
            );
            assert_eq!(scheduled_permission_mode(false, configured), configured);
        }
    }

    #[test]
    fn an_unattended_verdict_names_the_option_to_send() {
        let codex = content("command", codex_options());
        assert_eq!(unattended(&codex).option_id(), Some("decline"));
        let server = content(SERVER_NAME, codex_options());
        assert_eq!(unattended(&server).option_id(), Some("accept"));
        assert_eq!(Unattended::Cancel.option_id(), None);
    }

    #[test]
    fn unattended_agy_allows_report_tools_by_identity_never_by_title() {
        let options = vec![option("allow", "allow_once"), option("deny", "reject_once")];
        let spoof = content("ralph-worker-report: report", options.clone());
        let write = json!({"toolCall": {"name": "write_file"}});
        assert_eq!(
            unattended_acp(true, &write, &spoof),
            Unattended::Deny(&spoof.options[1])
        );
        assert_eq!(
            unattended_acp(false, &write, &spoof),
            Unattended::Allow(&spoof.options[0])
        );
        let report = content("report", options);
        let identity = json!({"toolCall": {"name": "mcp_ralph-worker_report"}});
        assert_eq!(
            unattended_acp(true, &identity, &report),
            Unattended::Allow(&report.options[0])
        );
        let bare = content("ralph-worker-report: report", Vec::new());
        assert_eq!(unattended_acp(true, &write, &bare), Unattended::Cancel);
    }

    #[test]
    fn unattended_allows_only_the_worker_report_tools() {
        let server = content(SERVER_NAME, codex_options());
        assert_eq!(unattended(&server), Unattended::Allow(&server.options[0]));
        for name in ["report", "set_status"] {
            let label = format!("{SERVER_NAME}-{name}: {name}");
            let acp = content(&label, vec![option("allow", "allow_once")]);
            assert_eq!(unattended(&acp), Unattended::Allow(&acp.options[0]));
        }
        let lookalike = content("ralph-worker-report: delete", codex_options());
        assert_eq!(
            unattended(&lookalike),
            Unattended::Deny(&lookalike.options[1])
        );
        let no_allow = content(SERVER_NAME, vec![option("decline", "reject_once")]);
        assert_eq!(
            unattended(&no_allow),
            Unattended::Deny(&no_allow.options[0])
        );
    }

    #[test]
    fn unattended_claude_allows_reads_and_denies_everything_else_with_a_report() {
        let answer = |ask: &ClaudePermission| {
            unattended_claude_answer(&WorkerId::parse("sched-1").unwrap(), "now", ask)
        };
        let read = answer(&claude_ask("mcp__mcptools__execute", json!({"code": "x"})));
        assert!(read.line.contains(r#""behavior":"allow""#), "{}", read.line);
        assert_eq!(read.report, None);
        for ask in [
            claude_ask("mcp__mcptools__execute", json!({"allowWrites": true})),
            claude_ask("mcp__mcptools__call_tool", json!({"allowSpend": true})),
            claude_ask("mcp__mcptools__execute", json!({"allowSpend": null})),
            claude_ask("mcp__mcptools__find_tools", json!({})),
            claude_ask("Bash", json!({"command": "ls"})),
        ] {
            let denied = answer(&ask);
            assert!(
                denied.line.contains(r#""behavior":"deny""#),
                "{}",
                denied.line
            );
            assert!(
                denied.line.contains(r#""request_id":"r1""#),
                "{}",
                denied.line
            );
            assert!(
                denied.line.contains("this worker runs unattended"),
                "{}",
                denied.line
            );
            assert!(!denied.line.contains("person"), "{}", denied.line);
            let report = denied.report.unwrap();
            assert_eq!(report.worker, "sched-1");
            assert!(report.text.contains(&ask.content.tool));
        }
    }

    #[test]
    fn a_denial_or_cancel_reports_blocked_and_an_allow_reports_nothing() {
        let bash = content("Bash", codex_options());
        for verdict in [Unattended::Deny(&bash.options[1]), Unattended::Cancel] {
            let report = verdict.denial_report(
                &WorkerId::parse("sched-1").unwrap(),
                "2026-11-24T10:00:00Z",
                &bash,
            );
            assert!(report.is_some(), "{verdict:?}");
            let report = report.unwrap();
            assert_eq!(report.worker, "sched-1");
            assert_eq!(report.at, "2026-11-24T10:00:00Z");
            assert_eq!(report.status, ReportKind::Worker(ReportStatus::Blocked));
            assert!(report.text.contains("Bash"));
            assert!(report.text.contains("do it"));
            assert!(report.text.contains("nobody was watching"));
        }
        let allow = Unattended::Allow(&bash.options[0]);
        assert_eq!(
            allow.denial_report(&WorkerId::parse("sched-1").unwrap(), "now", &bash),
            None
        );
    }

    #[test]
    fn a_scheduled_codex_sandbox_is_never_wider_than_workspace_write() {
        for (configured, want) in [
            (None, SandboxMode::WorkspaceWrite),
            (
                Some(SandboxMode::DangerFullAccess),
                SandboxMode::WorkspaceWrite,
            ),
            (
                Some(SandboxMode::WorkspaceWrite),
                SandboxMode::WorkspaceWrite,
            ),
            (Some(SandboxMode::ReadOnly), SandboxMode::ReadOnly),
        ] {
            assert_eq!(
                scheduled_sandbox_mode(true, configured),
                Some(want),
                "{configured:?}"
            );
            assert_eq!(scheduled_sandbox_mode(false, configured), configured);
        }
    }
}
