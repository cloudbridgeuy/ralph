use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaudeShell {
    #[default]
    Zsh,
    Bash,
}

impl ClaudeShell {
    pub fn program(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
        }
    }

    pub fn login_flags(self) -> &'static str {
        "-lic"
    }
}

pub fn resolve_claude_shell(
    config: Result<(ClaudeShell, Option<String>), &str>,
) -> (ClaudeShell, Option<String>) {
    match config {
        Ok((shell, warning)) => (shell, warning.map(|w| format!("Warning: {w}"))),
        Err(error) => (
            ClaudeShell::default(),
            Some(format!(
                "Warning: [claude] shell was skipped because the ralph config did not load ({error}). Used zsh instead."
            )),
        ),
    }
}

pub struct ClaudeWorkerArgs<'a> {
    pub store: &'a Path,
    pub mcp_config: &'a Path,
    pub settings: &'a Path,
    pub permission_mode: &'a str,
    pub attachments: &'a Path,
    pub task_dir: &'a Path,
    pub model: Option<&'a str>,
    pub scheduled: bool,
    pub shell: ClaudeShell,
    pub config_dir: Option<&'a Path>,
}

pub fn worker_launch(args: &ClaudeWorkerArgs) -> Vec<String> {
    let permission_mode = if args.scheduled {
        SCHEDULED_PERMISSION_MODE
    } else {
        args.permission_mode
    };
    let mut command: Vec<String> = [
        args.shell.program(),
        args.shell.login_flags(),
        r#"exec "$0" "$@""#,
    ]
    .map(str::to_string)
    .to_vec();
    let (key, inbox) = inbox_env(args.store);
    command.extend(["env".to_string(), format!("{key}={}", inbox.display())]);
    if let Some(dir) = args.config_dir {
        command.push(format!("CLAUDE_CONFIG_DIR={}", dir.to_string_lossy()));
    }
    command.extend(
        [
            "claude",
            "--permission-mode",
            permission_mode,
            "--mcp-config",
            &args.mcp_config.to_string_lossy(),
            "--settings",
            &args.settings.to_string_lossy(),
            "--add-dir",
            &args.attachments.to_string_lossy(),
            "--add-dir",
            &args.task_dir.to_string_lossy(),
        ]
        .map(str::to_string),
    );
    if args.scheduled {
        command.extend(disallowed_tool_args());
    }
    command.extend(["--append-system-prompt", worker_prompt(args.scheduled)].map(str::to_string));
    if let Some(model) = args.model {
        command.extend(["--model".to_string(), model.to_string()]);
    }
    command
}

pub fn worker_command(args: &ClaudeWorkerArgs) -> Vec<String> {
    let mut command = worker_launch(args);
    command.push(task_pointer(args.task_dir));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_args(model: Option<&str>) -> ClaudeWorkerArgs<'_> {
        ClaudeWorkerArgs {
            store: Path::new("/s"),
            mcp_config: Path::new("/s/workers/A.json"),
            settings: Path::new("/s/workers/A.settings.json"),
            permission_mode: "auto",
            attachments: Path::new("/s/attachments"),
            task_dir: Path::new("/s/workers/A.task"),
            model,
            scheduled: false,
            shell: ClaudeShell::Zsh,
            config_dir: None,
        }
    }

    #[test]
    fn claude_shell_resolves_config_or_warns_and_uses_zsh() {
        assert_eq!(
            resolve_claude_shell(Ok((ClaudeShell::Bash, None))),
            (ClaudeShell::Bash, None)
        );
        let (shell, warning) = resolve_claude_shell(Err("bad toml"));
        assert_eq!(shell, ClaudeShell::Zsh);
        let warning = warning.unwrap();
        assert!(warning.contains("bad toml") && warning.contains("zsh"));
    }

    #[test]
    fn claude_shell_names_program_and_login_flags() {
        assert_eq!(ClaudeShell::default(), ClaudeShell::Zsh);
        assert_eq!(ClaudeShell::Zsh.program(), "zsh");
        assert_eq!(ClaudeShell::Bash.program(), "bash");
        assert_eq!(ClaudeShell::Zsh.login_flags(), "-lic");
        assert_eq!(ClaudeShell::Bash.login_flags(), "-lic");
    }

    #[test]
    fn worker_command_runs_claude_through_bash_when_configured() {
        let args = worker_command(&ClaudeWorkerArgs {
            shell: ClaudeShell::Bash,
            ..claude_args(None)
        });
        assert_eq!(args[..2], ["bash", "-lic"]);
    }

    #[test]
    fn worker_command_ends_with_the_pointer_to_the_task_file() {
        let args = worker_command(&claude_args(None));
        assert_eq!(
            args.last().unwrap(),
            "Read your task from /s/workers/A.task/task.md"
        );
    }

    #[test]
    fn worker_command_adds_the_task_dir_once_beside_attachments() {
        let args = worker_command(&claude_args(None));
        let dirs: Vec<&String> = args
            .windows(2)
            .filter(|pair| pair[0] == "--add-dir")
            .map(|pair| &pair[1])
            .collect();
        assert_eq!(dirs, ["/s/attachments", "/s/workers/A.task"]);
    }

    #[test]
    fn worker_command_puts_model_before_the_task() {
        let args = worker_command(&claude_args(Some("opus")));
        assert_eq!(
            args[args.len() - 3..],
            [
                "--model",
                "opus",
                "Read your task from /s/workers/A.task/task.md"
            ]
        );
    }

    #[test]
    fn worker_command_omits_model_flag_when_unset() {
        let args = worker_command(&claude_args(None));
        assert!(!args.iter().any(|arg| arg == "--model"));
    }

    #[test]
    fn worker_command_sets_inbox_after_the_login_shell() {
        let args = worker_command(&claude_args(None));
        assert_eq!(&args[3..6], ["env", "RALPH_INBOX=/s/inbox", "claude"]);
    }

    #[test]
    fn worker_inbox_assignment_is_one_argument_for_both_login_shells() {
        for shell in [ClaudeShell::Zsh, ClaudeShell::Bash] {
            let args = worker_command(&ClaudeWorkerArgs {
                store: Path::new("/chat's store"),
                shell,
                ..claude_args(None)
            });
            assert_eq!(&args[..2], [shell.program(), "-lic"]);
            assert_eq!(
                &args[3..6],
                ["env", "RALPH_INBOX=/chat's store/inbox", "claude"]
            );
        }
    }

    #[test]
    fn worker_command_runs_claude_through_zsh_and_ends_with_the_task() {
        let args = worker_command(&claude_args(None));
        assert_eq!(args[..2], ["zsh", "-lic"]);
        assert_eq!(
            args[5..14],
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
        assert_eq!(
            args.last().unwrap(),
            "Read your task from /s/workers/A.task/task.md"
        );
    }

    #[test]
    fn worker_command_output_is_exactly_the_tmux_launch_vector() {
        let args = worker_command(&claude_args(None));
        assert_eq!(
            args,
            vec![
                "zsh".to_string(),
                "-lic".to_string(),
                r#"exec "$0" "$@""#.to_string(),
                "env".to_string(),
                "RALPH_INBOX=/s/inbox".to_string(),
                "claude".to_string(),
                "--permission-mode".to_string(),
                "auto".to_string(),
                "--mcp-config".to_string(),
                "/s/workers/A.json".to_string(),
                "--settings".to_string(),
                "/s/workers/A.settings.json".to_string(),
                "--add-dir".to_string(),
                "/s/attachments".to_string(),
                "--add-dir".to_string(),
                "/s/workers/A.task".to_string(),
                "--append-system-prompt".to_string(),
                WORKER_PROMPT.to_string(),
                "Read your task from /s/workers/A.task/task.md".to_string(),
            ]
        );
    }
}
