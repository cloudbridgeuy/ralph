use super::WORKER_PROMPT;
use std::ffi::OsString;

pub const SCHEDULED_WORKER_PROMPT: &str = "\
You work for one person on the task you were given, in your own git worktree. The task was scheduled and nobody is watching it run. Act, don't ask: make reasonable decisions yourself and keep going until the task is done. Never ask for confirmation or permission, and never ask questions in your reply text or with a built-in question tool; nobody reads them.

Never push, merge, open a pull request, or change Linear. Commit only on your own branch.

Finish with one report tool call: status done when the task is finished, or status blocked with the reason when you cannot go on. Then stop. Never wait for an answer and never send progress updates.

Call set_status with one sentence about what you are doing when you start and whenever your current activity changes; it is not a report and costs no turn.

A reference like [Image #N: <path>] in a message is an image file: read it at that path.";

pub fn worker_prompt(scheduled: bool) -> &'static str {
    if scheduled {
        SCHEDULED_WORKER_PROMPT
    } else {
        WORKER_PROMPT
    }
}

pub const SCHEDULED_PERMISSION_MODE: &str = "auto";

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
    use super::super::{worker_command, worktree_add_args, wt_switch_args, ClaudeWorkerArgs};
    use super::*;
    use std::path::Path;

    fn claude_args() -> ClaudeWorkerArgs<'static> {
        ClaudeWorkerArgs {
            mcp_config: Path::new("/s/workers/A.json"),
            settings: Path::new("/s/workers/A.settings.json"),
            permission_mode: "auto",
            attachments: Path::new("/s/attachments"),
            task: "do it",
            model: None,
            scheduled: false,
        }
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
    fn a_scheduled_worker_command_blocks_push_tools_before_a_flag() {
        let args = worker_command(&ClaudeWorkerArgs {
            scheduled: true,
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
        assert_eq!(args.last().unwrap(), "do it");
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
            ..claude_args()
        });
        let at = args
            .iter()
            .position(|a| a == "--append-system-prompt")
            .unwrap();
        assert_eq!(args[at + 1], SCHEDULED_WORKER_PROMPT);
    }
}
