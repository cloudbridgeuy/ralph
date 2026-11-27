use crate::program_status::{Stdout, TooLong};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::path::Path;

pub const TOKEN_ENV: &str = "OSC7577_TOKEN";
const MAX_SEQUENCE_BYTES: usize = 2048;

pub struct ResumeToken(String);

impl ResumeToken {
    pub fn parse(raw: &str) -> Option<Self> {
        let safe = raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
        (safe && (22..=64).contains(&raw.len())).then(|| Self(raw.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeSetting {
    Enabled,
    Disabled,
}

impl ResumeSetting {
    pub fn from_bool(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Off {
    Disabled,
    NotTty,
    NoToken,
    BadToken,
}

pub fn gate(
    setting: ResumeSetting,
    stdout: Stdout,
    token: Option<&str>,
) -> Result<ResumeToken, Off> {
    match (setting, stdout, token) {
        (ResumeSetting::Disabled, _, _) => Err(Off::Disabled),
        (_, Stdout::NotTty, _) => Err(Off::NotTty),
        (_, _, None) => Err(Off::NoToken),
        (_, _, Some(raw)) => ResumeToken::parse(raw).ok_or(Off::BadToken),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Argv(Vec<String>);

pub struct Cwd(String);

impl Cwd {
    pub fn parse(path: &Path) -> Result<Self, Unreportable> {
        reportable(path.as_os_str()).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreportable;

fn reportable(text: &OsStr) -> Result<String, Unreportable> {
    text.to_str()
        .filter(|text| !text.chars().any(char::is_control))
        .map(str::to_string)
        .ok_or(Unreportable)
}

#[derive(Clone, Copy)]
pub enum Action<'a> {
    Resume {
        argv: &'a Argv,
        cwd: Option<&'a Cwd>,
    },
    Clear,
}

pub struct Report<'a> {
    pub pid: u32,
    pub time: u64,
    pub action: Action<'a>,
}

pub fn encode(report: &Report<'_>, token: &ResumeToken) -> Result<String, TooLong> {
    let action = match report.action {
        Action::Resume { argv, cwd } => {
            let words = argv
                .0
                .iter()
                .map(|word| STANDARD.encode(word))
                .collect::<Vec<_>>()
                .join(",");
            let cwd = cwd
                .map(|cwd| format!(":cwd={}", STANDARD.encode(&cwd.0)))
                .unwrap_or_default();
            format!("argv={words}{cwd}")
        }
        Action::Clear => "clear=1".to_string(),
    };
    let body = format!("pid={}:time={}:{action}", report.pid, report.time);
    let mac = Sha256::digest(format!("{body}:{}", token.0))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let sequence = format!("\x1b]7577;{body}:mac={mac}\x1b\\");
    if sequence.len() > MAX_SEQUENCE_BYTES {
        Err(TooLong)
    } else {
        Ok(sequence)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Departure {
    UserQuit,
    Restart,
    Switch,
}

pub fn action_on_leave(departure: Departure) -> Option<Action<'static>> {
    matches!(departure, Departure::UserQuit).then_some(Action::Clear)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_TOKEN: &str = "dummy-token-replace-me";
    const SPEC_TIME: u64 = 1_795_543_200;

    fn encoded(pid: u32, action: Action<'_>) -> Result<String, TooLong> {
        let report = Report {
            pid,
            time: SPEC_TIME,
            action,
        };
        encode(&report, &ResumeToken::parse(SPEC_TOKEN).unwrap())
    }

    fn argv(words: &[&str]) -> Argv {
        Argv(words.iter().map(|word| word.to_string()).collect())
    }

    #[test]
    fn neovim_example_encodes_to_the_spec_bytes() {
        let argv = argv(&[
            "nvim",
            "-S",
            "/home/user/.local/state/nvim/sessions/8f2c1d.vim",
        ]);
        let action = Action::Resume {
            argv: &argv,
            cwd: None,
        };
        assert_eq!(
            encoded(74877, action).unwrap(),
            "\x1b]7577;pid=74877:time=1795543200:argv=bnZpbQ==,LVM=,L2hvbWUvdXNlci8ubG9jYWwvc3RhdGUvbnZpbS9zZXNzaW9ucy84ZjJjMWQudmlt:mac=9dbb99bfb856749a87563839d7ce9062818d78cf25909db93f309e0cd3b1909b\x1b\\"
        );
    }

    #[test]
    fn clear_example_encodes_to_the_spec_bytes() {
        assert_eq!(
            encoded(74877, Action::Clear).unwrap(),
            "\x1b]7577;pid=74877:time=1795543200:clear=1:mac=c30603369f98fef581a4098be889175b0c74f0641e49f8b25f4326cb38f97c93\x1b\\"
        );
    }

    #[test]
    fn lazygit_example_with_cwd_encodes_to_the_spec_bytes() {
        let argv = argv(&["lazygit"]);
        let cwd = Cwd::parse(Path::new("/home/user/src/project")).unwrap();
        let action = Action::Resume {
            argv: &argv,
            cwd: Some(&cwd),
        };
        assert_eq!(
            encoded(74877, action).unwrap(),
            "\x1b]7577;pid=74877:time=1795543200:argv=bGF6eWdpdA==:cwd=L2hvbWUvdXNlci9zcmMvcHJvamVjdA==:mac=35cb909805c380336efd3689a125acc774a9dec8422a1d21492ebec06ca758e1\x1b\\"
        );
    }

    #[test]
    fn wrapper_example_encodes_to_the_spec_bytes() {
        let argv = argv(&["resume-wrap", "ssh", "devbox"]);
        let action = Action::Resume {
            argv: &argv,
            cwd: None,
        };
        assert_eq!(
            encoded(90412, action).unwrap(),
            "\x1b]7577;pid=90412:time=1795543200:argv=cmVzdW1lLXdyYXA=,c3No,ZGV2Ym94:mac=e76149d0ba54a83570ffab0583867dacdfa23928ba9d18604f0badea88b62b65\x1b\\"
        );
    }

    #[test]
    fn a_token_outside_22_to_64_safe_characters_is_refused() {
        for good in ["a".repeat(22), "Z9_-".repeat(16)] {
            assert!(ResumeToken::parse(&good).is_some(), "{good}");
        }
        for bad in [
            String::new(),
            "a".repeat(21),
            "a".repeat(65),
            format!("{} ", "a".repeat(22)),
            format!("{}:", "a".repeat(22)),
            format!("{}\u{e9}", "a".repeat(22)),
        ] {
            assert!(ResumeToken::parse(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_gate_stays_closed_without_a_tty_a_token_or_the_setting() {
        let off =
            |setting, stdout, token: Option<&str>| gate(setting, stdout, token).map(drop).err();
        let on = ResumeSetting::Enabled;
        let token = Some(SPEC_TOKEN);
        assert_eq!(off(on, Stdout::Tty, token), None);
        assert_eq!(
            off(ResumeSetting::from_bool(false), Stdout::Tty, token),
            Some(Off::Disabled)
        );
        assert_eq!(off(on, Stdout::NotTty, token), Some(Off::NotTty));
        assert_eq!(off(on, Stdout::Tty, None), Some(Off::NoToken));
        assert_eq!(off(on, Stdout::Tty, Some("short")), Some(Off::BadToken));
    }

    #[test]
    fn a_report_over_2048_bytes_is_too_long() {
        let fits = argv(&[&"a".repeat(1400)]);
        let over = argv(&[&"a".repeat(1500)]);
        let sized = |argv| encoded(1, Action::Resume { argv, cwd: None });
        assert!(sized(&fits).unwrap().len() <= 2048);
        assert_eq!(sized(&over), Err(TooLong));
    }

    #[test]
    fn only_a_user_quit_sends_clear() {
        assert!(matches!(
            action_on_leave(Departure::UserQuit),
            Some(Action::Clear)
        ));
        assert!(action_on_leave(Departure::Restart).is_none());
        assert!(action_on_leave(Departure::Switch).is_none());
    }
}
