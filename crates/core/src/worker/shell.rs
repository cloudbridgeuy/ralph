use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerShell {
    Zsh,
    Bash,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellWrap {
    /// Direct login+interactive shell (`<shell> -lic 'exec "$0" "$@"' <argv...>`).
    Direct,
    /// Stdio-isolated login+interactive shell: routes rc stdout to stderr and rc stdin to
    /// `/dev/null`, then restores protocol fds 0/1, closes fds 3/4, and `exec`s the target.
    StdioIsolated,
}

impl WorkerShell {
    pub fn program(self) -> Option<&'static str> {
        let _ = self;
        None
    }

    pub fn as_str(self) -> &'static str {
        let _ = self;
        ""
    }

    pub fn login_flags(self) -> Option<&'static str> {
        let _ = self;
        None
    }

    pub fn from_passwd_shell(pw_shell: Option<&str>) -> (Self, Option<String>) {
        let _ = (pw_shell, Path::new(""));
        (Self::None, None)
    }
}

pub fn resolve_worker_shell(
    config: Result<(Option<WorkerShell>, Option<String>), &str>,
    passwd_shell: Option<&str>,
) -> (WorkerShell, Option<String>) {
    let _ = (config, passwd_shell);
    (WorkerShell::None, None)
}

pub fn with_env_prefix(envs: &[(&str, String)], argv: Vec<String>) -> Vec<String> {
    let _ = envs;
    argv
}

pub fn wrap_shell_command(shell: WorkerShell, mode: ShellWrap, argv: Vec<String>) -> Vec<String> {
    let _ = (shell, mode);
    argv
}

pub fn shell_preflight_command(shell: WorkerShell, program: &str) -> Option<Vec<String>> {
    let _ = (shell, program);
    None
}

pub fn shell_probe_command(shell: WorkerShell, program: &str, args: &[&str]) -> Vec<String> {
    let _ = (shell, program, args);
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "red test: WorkerShell program, as_str, and login_flags"]
    fn worker_shell_names_program_label_and_login_flags() {
        assert_eq!(WorkerShell::Zsh.program(), Some("zsh"));
        assert_eq!(WorkerShell::Bash.program(), Some("bash"));
        assert_eq!(WorkerShell::None.program(), None);

        assert_eq!(WorkerShell::Zsh.as_str(), "zsh");
        assert_eq!(WorkerShell::Bash.as_str(), "bash");
        assert_eq!(WorkerShell::None.as_str(), "none");

        assert_eq!(WorkerShell::Zsh.login_flags(), Some("-lic"));
        assert_eq!(WorkerShell::Bash.login_flags(), Some("-lic"));
        assert_eq!(WorkerShell::None.login_flags(), None);
    }

    #[test]
    #[ignore = "red test: WorkerShell passwd login shell mapping"]
    fn passwd_login_shell_maps_zsh_and_bash_and_warns_otherwise() {
        assert_eq!(
            WorkerShell::from_passwd_shell(Some("/bin/zsh")),
            (WorkerShell::Zsh, None)
        );
        assert_eq!(
            WorkerShell::from_passwd_shell(Some("/opt/homebrew/bin/zsh")),
            (WorkerShell::Zsh, None)
        );
        assert_eq!(
            WorkerShell::from_passwd_shell(Some("/bin/bash")),
            (WorkerShell::Bash, None)
        );
        assert_eq!(
            WorkerShell::from_passwd_shell(Some("/usr/local/bin/bash")),
            (WorkerShell::Bash, None)
        );
        let (shell, warning) = WorkerShell::from_passwd_shell(Some("/usr/bin/fish"));
        assert_eq!(shell, WorkerShell::None);
        let warning = warning.unwrap();
        assert!(warning.contains("fish") && warning.contains("[workers] shell"));

        for missing in [None, Some(""), Some("   ")] {
            let (shell, warning) = WorkerShell::from_passwd_shell(missing);
            assert_eq!(shell, WorkerShell::None);
            assert!(warning.unwrap().contains("[workers] shell"));
        }
    }

    #[test]
    #[ignore = "red test: resolve_worker_shell precedence and fallbacks"]
    fn resolve_worker_shell_uses_config_else_passwd_and_warns_on_errors() {
        assert_eq!(
            resolve_worker_shell(Ok((Some(WorkerShell::Bash), None)), Some("/bin/zsh")),
            (WorkerShell::Bash, None)
        );
        assert_eq!(
            resolve_worker_shell(Ok((Some(WorkerShell::None), None)), Some("/bin/fish")),
            (WorkerShell::None, None)
        );
        assert_eq!(
            resolve_worker_shell(Ok((None, None)), Some("/bin/zsh")),
            (WorkerShell::Zsh, None)
        );
        assert_eq!(
            resolve_worker_shell(Ok((None, None)), Some("/bin/bash")),
            (WorkerShell::Bash, None)
        );
        let (shell, warning) = resolve_worker_shell(Ok((None, None)), Some("/usr/bin/fish"));
        assert_eq!(shell, WorkerShell::None);
        assert!(warning.unwrap().starts_with("Warning: "));

        let (shell, warning) = resolve_worker_shell(
            Ok((
                None,
                Some("[workers] shell must be \"zsh\", \"bash\", or \"none\"".to_string()),
            )),
            Some("/bin/bash"),
        );
        assert_eq!(shell, WorkerShell::Bash);
        let warning = warning.unwrap();
        assert!(warning.contains("[workers] shell") && warning.contains("bash"));

        let (shell, warning) = resolve_worker_shell(Err("bad toml"), Some("/bin/zsh"));
        assert_eq!(shell, WorkerShell::Zsh);
        let warning = warning.unwrap();
        assert!(warning.contains("bad toml") && warning.contains("zsh"));
    }

    #[test]
    #[ignore = "red test: with_env_prefix"]
    fn with_env_prefix_prepends_env_only_when_non_empty() {
        let base = vec!["claude".to_string(), "-p".to_string()];
        assert_eq!(with_env_prefix(&[], base.clone()), base);
        assert_eq!(
            with_env_prefix(
                &[
                    ("RALPH_INBOX", "/chat's store/inbox".to_string()),
                    ("CLAUDE_CONFIG_DIR", "/c/alt dir".to_string()),
                ],
                base,
            ),
            vec![
                "env",
                "RALPH_INBOX=/chat's store/inbox",
                "CLAUDE_CONFIG_DIR=/c/alt dir",
                "claude",
                "-p",
            ]
        );
    }

    #[test]
    #[ignore = "red test: wrap_shell_command direct, stdio-isolated, none, tricky args"]
    fn wrap_shell_command_builds_direct_isolated_and_none_vectors() {
        let tricky = vec![
            "claude".to_string(),
            "hello world".to_string(),
            "it's $HOME `uname`".to_string(),
            r#"{"permissions":{"ask":["mcp__mcptools__execute"]}}"#.to_string(),
            "line1\nline2".to_string(),
            String::new(),
        ];

        assert_eq!(
            wrap_shell_command(WorkerShell::None, ShellWrap::Direct, tricky.clone()),
            tricky
        );
        assert_eq!(
            wrap_shell_command(WorkerShell::None, ShellWrap::StdioIsolated, tricky.clone()),
            tricky
        );
        assert!(wrap_shell_command(WorkerShell::Zsh, ShellWrap::Direct, Vec::new()).is_empty());

        let direct = wrap_shell_command(WorkerShell::Zsh, ShellWrap::Direct, tricky.clone());
        assert_eq!(direct[..3], ["zsh", "-lic", r#"exec "$0" "$@""#]);
        assert_eq!(&direct[3..], tricky.as_slice());

        for (shell, name) in [(WorkerShell::Zsh, "zsh"), (WorkerShell::Bash, "bash")] {
            let isolated = wrap_shell_command(shell, ShellWrap::StdioIsolated, tricky.clone());
            assert_eq!(
                isolated[..6],
                [
                    "/bin/sh",
                    "-c",
                    r#"exec 3>&1 4<&0 1>&2 </dev/null; exec "$0" "$@""#,
                    name,
                    "-lic",
                    r#"exec 1>&3 0<&4 3>&- 4<&-; exec "$0" "$@""#,
                ]
            );
            assert_eq!(&isolated[6..], tricky.as_slice());
        }
    }

    #[test]
    #[ignore = "red test: shell_preflight_command and shell_probe_command"]
    fn preflight_and_probe_commands_follow_worker_shell() {
        assert_eq!(shell_preflight_command(WorkerShell::None, "claude"), None);
        assert_eq!(
            shell_preflight_command(WorkerShell::Zsh, "claude"),
            Some(vec![
                "zsh".to_string(),
                "-lic".to_string(),
                r#"command -v "$0""#.to_string(),
                "claude".to_string(),
            ])
        );
        assert_eq!(
            shell_preflight_command(WorkerShell::Bash, "codex"),
            Some(vec![
                "bash".to_string(),
                "-lic".to_string(),
                r#"command -v "$0""#.to_string(),
                "codex".to_string(),
            ])
        );

        assert_eq!(
            shell_probe_command(WorkerShell::None, "claude", &["--version"]),
            vec!["claude", "--version"]
        );
        let wrapped = shell_probe_command(WorkerShell::Zsh, "claude", &["--version"]);
        assert_eq!(
            wrapped,
            vec![
                "/bin/sh",
                "-c",
                r#"exec 3>&1 4<&0 1>&2 </dev/null; exec "$0" "$@""#,
                "zsh",
                "-lic",
                r#"exec 1>&3 0<&4 3>&- 4<&-; exec "$0" "$@""#,
                "claude",
                "--version",
            ]
        );
    }
}
