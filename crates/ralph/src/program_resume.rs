use ralph_core::program_resume::{
    action_on_leave, encode, gate, Action, Cwd, Departure, Off, Report, ResumeSetting,
    ResumeTarget, ResumeToken, TOKEN_ENV,
};
use ralph_core::program_status::Stdout;
use ralph_server::warn::{warn, warn_if_err};
use std::io::{self, IsTerminal, Write};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct Resumer(Option<ResumeToken>);

impl Resumer {
    pub(crate) fn from_env(setting: ResumeSetting) -> Self {
        let token = std::env::var_os(TOKEN_ENV);
        let gated = gate(
            setting,
            Stdout::from_is_terminal(io::stdout().is_terminal()),
            token
                .as_ref()
                .map(|token| token.to_string_lossy())
                .as_deref(),
        );
        if matches!(gated, Err(Off::BadToken)) {
            warn(format!(
                "{TOKEN_ENV} is not a valid token. No resume command is reported."
            ));
        }
        Self(gated.ok())
    }

    pub(crate) fn publish(&self, target: &ResumeTarget) {
        let Some(token) = &self.0 else { return };
        let cwd = std::env::current_dir()
            .ok()
            .and_then(|dir| Cwd::parse(&dir).ok());
        let reported = match (target.argv(), cwd) {
            (Ok(argv), Some(cwd)) => send(
                token,
                Action::Resume {
                    argv: &argv,
                    cwd: Some(&cwd),
                },
            ),
            _ => false,
        };
        if !reported {
            warn("The resume command cannot be reported. Clearing the terminal's record.");
            send(token, Action::Clear);
        }
    }

    pub(crate) fn leave(&self, departure: Departure) {
        if let (Some(token), Some(action)) = (&self.0, action_on_leave(departure)) {
            send(token, action);
        }
    }
}

fn send(token: &ResumeToken, action: Action<'_>) -> bool {
    let report = Report {
        pid: std::process::id(),
        time: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        action,
    };
    let Ok(sequence) = encode(&report, token) else {
        return false;
    };
    let mut out = io::stdout().lock();
    warn_if_err(
        out.write_all(sequence.as_bytes())
            .and_then(|()| out.flush()),
        "Failed to report the resume command",
    );
    true
}
