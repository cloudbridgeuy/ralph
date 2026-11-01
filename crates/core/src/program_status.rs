use base64::engine::general_purpose::STANDARD;
use base64::Engine;

pub const APP: &str = "ralph";
const MAX_SEQUENCE_BYTES: usize = 4096;
const MAX_MSG_BYTES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Question,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Question => "question",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    Done,
    Blocked(Kind),
    Clear,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Working => "working",
            State::Done => "done",
            State::Blocked(_) => "blocked",
            State::Clear => "clear",
        }
    }

    fn kind(self) -> Option<Kind> {
        match self {
            State::Blocked(kind) => Some(kind),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub state: State,
    pub msg: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLong {
    pub bytes: usize,
}

fn clean_msg(msg: &str) -> String {
    let stripped: String = msg.chars().filter(|c| !c.is_control()).collect();
    let mut end = stripped.len().min(MAX_MSG_BYTES);
    while !stripped.is_char_boundary(end) {
        end -= 1;
    }
    stripped[..end].to_string()
}

fn bounded(sequence: String) -> Result<String, TooLong> {
    if sequence.len() > MAX_SEQUENCE_BYTES {
        Err(TooLong {
            bytes: sequence.len(),
        })
    } else {
        Ok(sequence)
    }
}

pub fn encode(report: &Report) -> Result<String, TooLong> {
    let kind = report
        .state
        .kind()
        .map(|kind| format!(":kind={}", kind.as_str()))
        .unwrap_or_default();
    let msg = report
        .msg
        .as_deref()
        .map(|msg| format!(":msg={}", STANDARD.encode(clean_msg(msg))))
        .unwrap_or_default();
    bounded(format!(
        "\x1b]7501;state={}{kind}:app={APP}{msg}\x07",
        report.state.as_str()
    ))
}

pub fn tmux_wrap(sequence: &str) -> String {
    format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(state: State, msg: Option<&str>) -> Report {
        Report {
            state,
            msg: msg.map(str::to_string),
        }
    }

    fn decoded_msg(sequence: &str) -> String {
        let encoded = sequence
            .split(':')
            .find_map(|field| field.strip_prefix("msg="))
            .unwrap()
            .trim_end_matches('\x07');
        String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap()
    }

    #[test]
    fn working_without_msg() {
        assert_eq!(
            encode(&report(State::Working, None)).unwrap(),
            "\x1b]7501;state=working:app=ralph\x07"
        );
    }

    #[test]
    fn blocked_question_carries_kind_and_msg() {
        assert_eq!(
            encode(&report(
                State::Blocked(Kind::Question),
                Some("2 worker reports")
            ))
            .unwrap(),
            "\x1b]7501;state=blocked:kind=question:app=ralph:msg=MiB3b3JrZXIgcmVwb3J0cw==\x07"
        );
    }

    #[test]
    fn clear_has_no_id() {
        assert_eq!(
            encode(&report(State::Clear, None)).unwrap(),
            "\x1b]7501;state=clear:app=ralph\x07"
        );
    }

    #[test]
    fn kind_is_only_emitted_with_blocked() {
        [State::Idle, State::Working, State::Done, State::Clear]
            .into_iter()
            .for_each(|state| {
                assert!(!encode(&report(state, None)).unwrap().contains("kind="));
            });
    }

    #[test]
    fn msg_round_trips_utf8() {
        let sequence = encode(&report(State::Working, Some("holding w\u{e9}\u{1f600}"))).unwrap();
        assert_eq!(decoded_msg(&sequence), "holding w\u{e9}\u{1f600}");
    }

    #[test]
    fn control_chars_and_newlines_are_stripped() {
        let sequence = encode(&report(State::Working, Some("a\nb\x07c\x1bd\te\r"))).unwrap();
        assert_eq!(decoded_msg(&sequence), "abcde");
    }

    #[test]
    fn msg_is_cut_at_a_char_boundary() {
        let long = format!("a{}", "\u{e9}".repeat(1100));
        let sequence = encode(&report(State::Working, Some(&long))).unwrap();
        let decoded = decoded_msg(&sequence);
        assert_eq!(decoded.len(), 2047);
        assert!(decoded.starts_with('a'));
        assert!(decoded.ends_with('\u{e9}'));
    }

    #[test]
    fn msg_at_the_limit_is_kept_whole() {
        let exact = "a".repeat(2048);
        let sequence = encode(&report(State::Working, Some(&exact))).unwrap();
        assert_eq!(decoded_msg(&sequence), exact);
    }

    #[test]
    fn largest_report_fits_the_sequence_limit() {
        let sequence = encode(&report(
            State::Blocked(Kind::Question),
            Some(&"\u{e9}".repeat(5000)),
        ))
        .unwrap();
        assert!(sequence.len() <= 4096);
    }

    #[test]
    fn sequences_over_the_limit_are_rejected() {
        assert_eq!(bounded("x".repeat(4097)), Err(TooLong { bytes: 4097 }));
        assert_eq!(bounded("x".repeat(4096)), Ok("x".repeat(4096)));
    }

    #[test]
    fn tmux_wrap_doubles_escapes() {
        assert_eq!(
            tmux_wrap("\x1b]7501;state=idle:app=ralph\x07"),
            "\x1bPtmux;\x1b\x1b]7501;state=idle:app=ralph\x07\x1b\\"
        );
    }
}
