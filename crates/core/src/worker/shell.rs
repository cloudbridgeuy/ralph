use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::Path;

const ISOLATE_STDIO: &str = r#"exec 3>&1 4<&0 1>&2 </dev/null; exec "$0" "$@""#;
const ZSH_RESTORE_STDIO: &str = r#"exec 1>&3 0<&4 3>&- 4<&-; exec "$0" "$@""#;
// Bash manual, "Bash Startup Files": an interactive login shell reads /etc/profile, then the
// first readable file of ~/.bash_profile, ~/.bash_login and ~/.profile. bash 3.2 closes
// inherited fds >= 3 under -l, so a non-login interactive bash sources the same files here.
const BASH_LOGIN_RESTORE_STDIO: &str = r#"ralph_argv=("$0" "$@"); [ -r /etc/profile ] && . /etc/profile; for ralph_rc in ~/.bash_profile ~/.bash_login ~/.profile; do if [ -r "$ralph_rc" ]; then . "$ralph_rc"; break; fi; done; exec 1>&3 0<&4 3>&- 4<&-; exec "${ralph_argv[@]}""#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerShell {
    Zsh,
    Bash,
    None,
}

impl WorkerShell {
    fn name(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildSession {
    Group,
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginShell {
    Zsh(String),
    Bash(String),
    Direct,
}

impl LoginShell {
    fn of(kind: WorkerShell, program: &str) -> Self {
        match kind {
            WorkerShell::Zsh => Self::Zsh(program.to_string()),
            WorkerShell::Bash => Self::Bash(program.to_string()),
            WorkerShell::None => Self::Direct,
        }
    }

    pub fn session(&self) -> ChildSession {
        match self {
            Self::Zsh(_) | Self::Bash(_) => ChildSession::Detached,
            Self::Direct => ChildSession::Group,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Zsh(program) | Self::Bash(program) => program,
            Self::Direct => "none",
        }
    }
}

fn passwd_shell(passwd: Option<&str>) -> Result<(WorkerShell, &str), String> {
    let path = passwd
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or("could not read the login shell from passwd")?;
    match Path::new(path).file_name().and_then(|name| name.to_str()) {
        Some("zsh") => Ok((WorkerShell::Zsh, path)),
        Some("bash") => Ok((WorkerShell::Bash, path)),
        _ => Err(format!("login shell {path} from passwd is not zsh or bash")),
    }
}

fn passwd_default(passwd: Option<&str>) -> (LoginShell, Option<String>) {
    match passwd_shell(passwd) {
        Ok((kind, path)) => (LoginShell::of(kind, path), None),
        Err(reason) => (
            LoginShell::Direct,
            Some(format!(
                "{reason}; workers start without a shell; set [workers] shell to \"zsh\", \"bash\" or \"none\""
            )),
        ),
    }
}

fn configured(kind: WorkerShell, passwd: Option<&str>) -> LoginShell {
    let program = match passwd_shell(passwd) {
        Ok((found, path)) if found == kind => path,
        _ => kind.name(),
    };
    LoginShell::of(kind, program)
}

pub fn resolve_worker_shell(
    config: Result<(Option<WorkerShell>, Option<String>), &str>,
    passwd: Option<&str>,
) -> (LoginShell, Option<String>) {
    let skipped = match config {
        Ok((Some(kind), _)) => return (configured(kind, passwd), None),
        Ok((None, None)) => return passwd_default(passwd),
        Ok((None, Some(invalid))) => invalid,
        Err(error) => {
            format!("[workers] shell was skipped because the ralph config did not load ({error})")
        }
    };
    let (shell, warning) = passwd_default(passwd);
    let outcome = warning.unwrap_or_else(|| format!("using {}", shell.label()));
    (shell, Some(format!("{skipped}; {outcome}")))
}

pub fn shell_wrapped_argv(shell: &LoginShell, argv: Vec<OsString>) -> Vec<OsString> {
    let prefix: Vec<&str> = match shell {
        LoginShell::Direct => return argv,
        LoginShell::Zsh(program) => {
            vec![
                "/bin/sh",
                "-c",
                ISOLATE_STDIO,
                program,
                "-lic",
                ZSH_RESTORE_STDIO,
            ]
        }
        LoginShell::Bash(program) => vec![
            "/bin/sh",
            "-c",
            ISOLATE_STDIO,
            program,
            "--norc",
            "--noprofile",
            "-ic",
            BASH_LOGIN_RESTORE_STDIO,
        ],
    };
    prefix.into_iter().map(OsString::from).chain(argv).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNSET: Result<(Option<WorkerShell>, Option<String>), &str> = Ok((None, None));

    fn zsh(path: &str) -> LoginShell {
        LoginShell::Zsh(path.to_string())
    }

    fn bash(path: &str) -> LoginShell {
        LoginShell::Bash(path.to_string())
    }

    #[test]
    fn unset_config_takes_the_passwd_login_shell_path() {
        assert_eq!(
            resolve_worker_shell(UNSET, Some("/bin/zsh")),
            (zsh("/bin/zsh"), None)
        );
        assert_eq!(
            resolve_worker_shell(UNSET, Some("/opt/homebrew/bin/bash")),
            (bash("/opt/homebrew/bin/bash"), None)
        );
        for passwd in [Some("/usr/bin/fish"), None, Some(""), Some("  ")] {
            let (shell, warning) = resolve_worker_shell(UNSET, passwd);
            assert_eq!(shell, LoginShell::Direct);
            let warning = warning.unwrap();
            assert!(warning.contains("passwd") && warning.contains("[workers] shell"));
        }
    }

    #[test]
    fn configured_shell_reuses_the_passwd_path_only_when_it_is_the_same_shell() {
        let set = |shell| Ok((Some(shell), None));
        assert_eq!(
            resolve_worker_shell(set(WorkerShell::Bash), Some("/bin/bash")),
            (bash("/bin/bash"), None)
        );
        assert_eq!(
            resolve_worker_shell(set(WorkerShell::Bash), Some("/bin/zsh")),
            (bash("bash"), None)
        );
        assert_eq!(
            resolve_worker_shell(set(WorkerShell::Zsh), None),
            (zsh("zsh"), None)
        );
        assert_eq!(
            resolve_worker_shell(set(WorkerShell::None), Some("/bin/zsh")),
            (LoginShell::Direct, None)
        );
    }

    #[test]
    fn invalid_or_unloaded_config_falls_back_to_passwd_with_a_warning() {
        let invalid = Ok((None, Some("[workers] shell must be zsh".to_string())));
        let (shell, warning) = resolve_worker_shell(invalid, Some("/bin/zsh"));
        assert_eq!(shell, zsh("/bin/zsh"));
        assert_eq!(
            warning.as_deref(),
            Some("[workers] shell must be zsh; using /bin/zsh")
        );

        let (shell, warning) = resolve_worker_shell(Err("bad toml"), Some("/bin/bash"));
        assert_eq!(shell, bash("/bin/bash"));
        let warning = warning.unwrap();
        assert!(warning.contains("bad toml") && warning.ends_with("using /bin/bash"));

        let (shell, warning) = resolve_worker_shell(Err("bad toml"), None);
        assert_eq!(shell, LoginShell::Direct);
        assert!(warning.unwrap().contains("passwd"));
    }

    #[test]
    fn wrapped_children_leave_the_terminal_and_direct_children_keep_a_group() {
        assert_eq!(zsh("/bin/zsh").session(), ChildSession::Detached);
        assert_eq!(bash("bash").session(), ChildSession::Detached);
        assert_eq!(LoginShell::Direct.session(), ChildSession::Group);
    }

    #[test]
    fn labels_name_the_program_or_none() {
        assert_eq!(zsh("/bin/zsh").label(), "/bin/zsh");
        assert_eq!(bash("bash").label(), "bash");
        assert_eq!(LoginShell::Direct.label(), "none");
    }

    fn argv(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn wrapped_argv_isolates_stdio_and_passes_the_target_argv_untouched() {
        let target = argv(&[
            "claude",
            "hello world",
            "it's $HOME `uname`",
            r#"{"permissions":{"ask":["x"]}}"#,
            "line1\nline2",
            "",
        ]);
        assert_eq!(
            shell_wrapped_argv(&LoginShell::Direct, target.clone()),
            target
        );

        let wrapped = shell_wrapped_argv(&zsh("/bin/zsh"), target.clone());
        assert_eq!(
            wrapped[..6],
            argv(&[
                "/bin/sh",
                "-c",
                r#"exec 3>&1 4<&0 1>&2 </dev/null; exec "$0" "$@""#,
                "/bin/zsh",
                "-lic",
                r#"exec 1>&3 0<&4 3>&- 4<&-; exec "$0" "$@""#,
            ])[..]
        );
        assert_eq!(wrapped[6..], target[..]);

        let wrapped = shell_wrapped_argv(&bash("/bin/bash"), target.clone());
        assert_eq!(
            wrapped[..7],
            argv(&[
                "/bin/sh",
                "-c",
                r#"exec 3>&1 4<&0 1>&2 </dev/null; exec "$0" "$@""#,
                "/bin/bash",
                "--norc",
                "--noprofile",
                "-ic",
            ])[..]
        );
        let script = wrapped[7].to_str().unwrap();
        assert!(script.starts_with(r#"ralph_argv=("$0" "$@"); [ -r /etc/profile ] && . /etc/profile; for ralph_rc in ~/.bash_profile ~/.bash_login ~/.profile; do"#));
        assert!(script.ends_with(r#"exec 1>&3 0<&4 3>&- 4<&-; exec "${ralph_argv[@]}""#));
        assert_eq!(wrapped[8..], target[..]);
    }
}
