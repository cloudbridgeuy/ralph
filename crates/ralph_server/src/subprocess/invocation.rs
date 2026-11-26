use std::ffi::OsString;
use std::io;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: &'static str,
    pub args: Vec<OsString>,
    pub env: Vec<(&'static str, OsString)>,
}

impl Invocation {
    pub(super) fn command(&self) -> Command {
        let mut command = Command::new(self.program);
        command
            .args(&self.args)
            .envs(self.env.iter().map(|(key, value)| (key, value)));
        command
    }

    pub(super) fn spawn_error(&self, error: io::Error) -> io::Error {
        io::Error::new(error.kind(), format!("{}: {error}", self.program))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn a_command_gets_each_argument_and_variable_as_written() {
        let invocation = Invocation {
            program: "claude",
            args: vec!["--model".into(), "x; id $(id)".into()],
            env: vec![("RALPH_INBOX", "/a b".into())],
        };
        let command = invocation.command();
        assert_eq!(command.get_program(), "claude");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--model", "x; id $(id)"]
        );
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            [(OsStr::new("RALPH_INBOX"), Some(OsStr::new("/a b")))]
        );
    }

    #[test]
    fn a_spawn_error_names_the_program_and_keeps_the_kind() {
        let invocation = Invocation {
            program: "claude",
            args: Vec::new(),
            env: Vec::new(),
        };
        let error = invocation.spawn_error(io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().starts_with("claude: "), "{error}");
    }
}
