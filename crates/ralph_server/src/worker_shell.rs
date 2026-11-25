use crate::config::AppConfig;
use crate::transport::stdio::spawn_piped;
use crate::warn::warn_once;
use crate::wire::store::user_login_shell;
use ralph_core::worker::{resolve_worker_shell, shell_wrapped_argv, LoginShell};
use std::ffi::OsStr;
use std::io;
use std::iter;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

pub(crate) fn worker_shell() -> LoginShell {
    let config = AppConfig::load().map_err(|error| error.to_string());
    let (shell, warning) = resolve_worker_shell(
        config
            .as_ref()
            .map(|config| config.workers.shell_setting())
            .map_err(String::as_str),
        user_login_shell().as_deref(),
    );
    if let Some(warning) = warning {
        warn_once("workers-shell", warning);
    }
    shell
}

pub(crate) fn spawn_in_shell(
    command: Command,
) -> io::Result<(Child, ChildStdin, ChildStdout, ChildStderr)> {
    let shell = worker_shell();
    spawn_piped(in_shell(&shell, command), shell.session())
}

pub(crate) fn in_shell(shell: &LoginShell, command: Command) -> Command {
    let argv = iter::once(command.get_program())
        .chain(command.get_args())
        .map(OsStr::to_os_string)
        .collect();
    let wrapped = shell_wrapped_argv(shell, argv);
    let Some((program, args)) = wrapped.split_first() else {
        return command;
    };
    let mut shelled = Command::new(program);
    shelled.args(args);
    if let Some(dir) = command.get_current_dir() {
        shelled.current_dir(dir);
    }
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => shelled.env(key, value),
            None => shelled.env_remove(key),
        };
    }
    shelled
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn in_shell_wraps_argv_and_keeps_dir_and_env_changes() {
        let mut command = Command::new("claude");
        command
            .args(["-p", "it's $HOME"])
            .current_dir("/tmp/work tree")
            .env("RALPH_INBOX", "/store/inbox")
            .env_remove("CLAUDECODE");
        let shelled = in_shell(&LoginShell::Zsh("/bin/zsh".to_string()), command);
        assert_eq!(shelled.get_program(), "/bin/sh");
        let args: Vec<&OsStr> = shelled.get_args().collect();
        assert_eq!(args[..1], [OsStr::new("-c")]);
        assert_eq!(args[2..4], [OsStr::new("/bin/zsh"), OsStr::new("-lic")]);
        assert_eq!(
            args[5..],
            [
                OsStr::new("claude"),
                OsStr::new("-p"),
                OsStr::new("it's $HOME")
            ]
        );
        assert_eq!(shelled.get_current_dir(), Some(Path::new("/tmp/work tree")));
        let envs: Vec<_> = shelled.get_envs().collect();
        assert_eq!(
            envs,
            [
                (OsStr::new("CLAUDECODE"), None),
                (OsStr::new("RALPH_INBOX"), Some(OsStr::new("/store/inbox"))),
            ]
        );
    }

    #[test]
    fn in_shell_leaves_a_direct_command_as_built() {
        let mut command = Command::new("codex");
        command.args(["app-server"]).current_dir("/w");
        let direct = in_shell(&LoginShell::Direct, command);
        assert_eq!(direct.get_program(), "codex");
        assert_eq!(direct.get_args().collect::<Vec<_>>(), ["app-server"]);
        assert_eq!(direct.get_current_dir(), Some(Path::new("/w")));
    }
}
