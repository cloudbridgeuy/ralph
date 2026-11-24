use serde::{Deserialize, Serialize};
use std::path::Path;

const DIRECT_EXEC: &str = r#"exec "$0" "$@""#;
const ISOLATE_OUTER: &str = r#"exec 3>&1 4<&0 1>&2 </dev/null; exec "$0" "$@""#;
const ISOLATE_INNER: &str = r#"exec 1>&3 0<&4 3>&- 4<&-; exec "$0" "$@""#;
const PREFLIGHT_CHECK: &str = r#"command -v "$0""#;

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
        match self {
            Self::Zsh => Some("zsh"),
            Self::Bash => Some("bash"),
            Self::None => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::None => "none",
        }
    }

    pub fn login_flags(self) -> Option<&'static str> {
        match self {
            Self::Zsh | Self::Bash => Some("-lic"),
            Self::None => None,
        }
    }

    pub fn from_passwd_shell(pw_shell: Option<&str>) -> (Self, Option<String>) {
        let name = pw_shell
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str());
        match name {
            Some("zsh") => (Self::Zsh, None),
            Some("bash") => (Self::Bash, None),
            Some(other) => (
                Self::None,
                Some(format!(
                    "login shell '{other}' from passwd is not supported; running workers directly without a shell; set [workers] shell to \"zsh\", \"bash\", or \"none\""
                )),
            ),
            None => (
                Self::None,
                Some(
                    "could not determine login shell from passwd; running workers directly without a shell; set [workers] shell to \"zsh\", \"bash\", or \"none\""
                        .to_string(),
                ),
            ),
        }
    }
}

pub fn resolve_worker_shell(
    config: Result<(Option<WorkerShell>, Option<String>), &str>,
    passwd_shell: Option<&str>,
) -> (WorkerShell, Option<String>) {
    match config {
        Ok((Some(shell), _)) => (shell, None),
        Ok((None, None)) => {
            let (shell, warning) = WorkerShell::from_passwd_shell(passwd_shell);
            (shell, warning.map(|w| format!("Warning: {w}")))
        }
        Ok((None, Some(bad))) => {
            let (fallback, _) = WorkerShell::from_passwd_shell(passwd_shell);
            (
                fallback,
                Some(format!("Warning: {bad}; using \"{}\"", fallback.as_str())),
            )
        }
        Err(error) => {
            let (fallback, _) = WorkerShell::from_passwd_shell(passwd_shell);
            (
                fallback,
                Some(format!(
                    "Warning: [workers] shell was skipped because the ralph config did not load ({error}). Used \"{}\" instead.",
                    fallback.as_str()
                )),
            )
        }
    }
}

pub fn with_env_prefix(envs: &[(&str, String)], argv: Vec<String>) -> Vec<String> {
    if envs.is_empty() {
        return argv;
    }
    std::iter::once("env".to_string())
        .chain(envs.iter().map(|(key, value)| format!("{key}={value}")))
        .chain(argv)
        .collect()
}

pub fn wrap_shell_command(shell: WorkerShell, mode: ShellWrap, argv: Vec<String>) -> Vec<String> {
    if argv.is_empty() {
        return argv;
    }
    let (Some(program), Some(flags)) = (shell.program(), shell.login_flags()) else {
        return argv;
    };
    let prefix: Vec<String> = match mode {
        ShellWrap::Direct => [program, flags, DIRECT_EXEC].map(str::to_string).to_vec(),
        ShellWrap::StdioIsolated => [
            "/bin/sh",
            "-c",
            ISOLATE_OUTER,
            program,
            flags,
            ISOLATE_INNER,
        ]
        .map(str::to_string)
        .to_vec(),
    };
    prefix.into_iter().chain(argv).collect()
}

pub fn shell_preflight_command(shell: WorkerShell, program: &str) -> Option<Vec<String>> {
    let (shell_prog, flags) = shell.program().zip(shell.login_flags())?;
    Some(
        [shell_prog, flags, PREFLIGHT_CHECK, program]
            .map(str::to_string)
            .to_vec(),
    )
}

pub fn shell_probe_command(shell: WorkerShell, program: &str, args: &[&str]) -> Vec<String> {
    let argv: Vec<String> = std::iter::once(program)
        .chain(args.iter().copied())
        .map(str::to_string)
        .collect();
    wrap_shell_command(shell, ShellWrap::StdioIsolated, argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
