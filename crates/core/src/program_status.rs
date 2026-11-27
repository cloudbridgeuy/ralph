use crate::chat_status::Status;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

pub const APP: &str = "ralph";
pub const REX_BLOCK_ENV: &str = "REX_BLOCK";
const MAX_SEQUENCE_BYTES: usize = 4096;
const MAX_MSG_BYTES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub state: State,
    pub msg: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLong;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Turn,
    Cell,
    Replied,
    Prompt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RexBlock(String);

impl RexBlock {
    pub fn parse(raw: &str) -> Option<Self> {
        (!raw.is_empty()).then(|| Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RexSplit<'a> {
    pub block: &'a RexBlock,
    pub label: &'a str,
    pub cwd: &'a str,
}

pub fn rex_split_argv(split: RexSplit<'_>, command: Vec<String>) -> Vec<String> {
    [
        "split",
        "-b",
        split.block.as_str(),
        "--label",
        split.label,
        "-c",
        split.cwd,
        "--interactive",
        "--",
    ]
    .map(str::to_string)
    .into_iter()
    .chain(command)
    .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusSetting {
    Enabled,
    Disabled,
}

impl StatusSetting {
    pub fn from_bool(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stdout {
    Tty,
    NotTty,
}

impl Stdout {
    pub fn from_is_terminal(is_terminal: bool) -> Self {
        if is_terminal {
            Self::Tty
        } else {
            Self::NotTty
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RexPane(());

impl RexPane {
    pub fn report(self, report: &Report) -> Result<String, TooLong> {
        encode(report)
    }
}

pub fn rex_pane(setting: StatusSetting, stdout: Stdout, rex: Option<&RexBlock>) -> Option<RexPane> {
    matches!(
        (setting, stdout, rex),
        (StatusSetting::Enabled, Stdout::Tty, Some(_))
    )
    .then_some(RexPane(()))
}

fn unread_msg(unread: usize) -> String {
    match unread {
        1 => "1 worker report".to_string(),
        n => format!("{n} worker reports"),
    }
}

pub fn report_for(phase: Phase, status: &Status) -> Report {
    let (state, msg) = if let Some(hold) = &status.hold {
        (State::Working, Some(format!("holding {}", hold.label())))
    } else if matches!(phase, Phase::Turn | Phase::Cell) {
        (State::Working, None)
    } else if status.unread > 0 {
        (
            State::Blocked(Kind::Question),
            Some(unread_msg(status.unread)),
        )
    } else if phase == Phase::Replied {
        (State::Done, None)
    } else {
        (State::Idle, None)
    };
    Report { state, msg }
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
        Err(TooLong)
    } else {
        Ok(sequence)
    }
}

fn encode(report: &Report) -> Result<String, TooLong> {
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
    use crate::chat_hold::HoldFilter;
    use crate::chat_status::{WorkerActivity, WorkerState};

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
    fn rex_pane_needs_config_tty_and_non_empty_rex_block() {
        let block = RexBlock::parse("blk-1").unwrap();
        let pane = rex_pane(StatusSetting::Enabled, Stdout::Tty, Some(&block)).unwrap();
        assert_eq!(std::mem::size_of::<RexPane>(), 0);
        assert_eq!(
            pane.report(&report(State::Working, None)).unwrap(),
            "\x1b]7501;state=working:app=ralph\x07"
        );
        assert_eq!(
            rex_pane(StatusSetting::Disabled, Stdout::Tty, Some(&block)),
            None
        );
        assert_eq!(
            rex_pane(StatusSetting::Enabled, Stdout::NotTty, Some(&block)),
            None
        );
        assert_eq!(rex_pane(StatusSetting::Enabled, Stdout::Tty, None), None);
        assert_eq!(RexBlock::parse(""), None);
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
        assert_eq!(bounded("x".repeat(4097)), Err(TooLong));
        assert_eq!(bounded("x".repeat(4096)), Ok("x".repeat(4096)));
    }

    #[test]
    fn tmux_wrap_doubles_escapes() {
        assert_eq!(
            tmux_wrap("\x1b]7501;state=idle:app=ralph\x07"),
            "\x1bPtmux;\x1b\x1b]7501;state=idle:app=ralph\x07\x1b\\"
        );
    }

    fn status_with(unread: usize, hold: Option<HoldFilter>) -> Status {
        Status {
            unread,
            hold,
            ..Status::default()
        }
    }

    fn running_worker() -> WorkerState {
        WorkerState {
            id: "w".into(),
            state: WorkerActivity::Running,
        }
    }

    fn expect(state: State, msg: Option<&str>) -> Report {
        report(state, msg)
    }

    #[test]
    fn report_for_follows_the_priority_table() {
        let question = State::Blocked(Kind::Question);
        let hold_w1 = Some(HoldFilter::Source("w1".into()));
        let cases = [
            (
                Phase::Prompt,
                status_with(0, None),
                expect(State::Idle, None),
            ),
            (
                Phase::Turn,
                status_with(0, None),
                expect(State::Working, None),
            ),
            (
                Phase::Replied,
                status_with(0, None),
                expect(State::Done, None),
            ),
            (
                Phase::Prompt,
                status_with(0, hold_w1.clone()),
                expect(State::Working, Some("holding w1")),
            ),
            (
                Phase::Prompt,
                status_with(0, Some(HoldFilter::Any)),
                expect(State::Working, Some("holding anyone")),
            ),
            (
                Phase::Prompt,
                status_with(4, hold_w1.clone()),
                expect(State::Working, Some("holding w1")),
            ),
            (
                Phase::Turn,
                status_with(0, hold_w1),
                expect(State::Working, Some("holding w1")),
            ),
            (
                Phase::Prompt,
                status_with(1, None),
                expect(question, Some("1 worker report")),
            ),
            (
                Phase::Prompt,
                status_with(3, None),
                expect(question, Some("3 worker reports")),
            ),
            (
                Phase::Replied,
                status_with(2, None),
                expect(question, Some("2 worker reports")),
            ),
            (
                Phase::Turn,
                status_with(5, None),
                expect(State::Working, None),
            ),
            (
                Phase::Cell,
                status_with(0, None),
                expect(State::Working, None),
            ),
            (
                Phase::Cell,
                status_with(2, None),
                expect(State::Working, None),
            ),
            (
                Phase::Cell,
                status_with(0, Some(HoldFilter::Any)),
                expect(State::Working, Some("holding anyone")),
            ),
        ];
        for (phase, status, expected) in cases {
            assert_eq!(report_for(phase, &status), expected, "{phase:?} {status:?}");
        }
    }

    #[test]
    fn nap_reports_idle() {
        let status = Status {
            napping: true,
            ..Status::default()
        };
        assert_eq!(
            report_for(Phase::Prompt, &status),
            expect(State::Idle, None)
        );
    }

    #[test]
    fn running_workers_without_a_turn_report_idle() {
        let status = Status {
            workers: vec![running_worker()],
            ..Status::default()
        };
        assert_eq!(
            report_for(Phase::Prompt, &status),
            expect(State::Idle, None)
        );
    }

    #[test]
    fn running_workers_after_a_reply_report_done() {
        let status = Status {
            workers: vec![running_worker()],
            ..Status::default()
        };
        assert_eq!(
            report_for(Phase::Replied, &status),
            expect(State::Done, None)
        );
    }

    #[test]
    fn a_report_arriving_mid_turn_stays_working() {
        let before = status_with(0, None);
        let after = status_with(1, None);
        assert_eq!(
            report_for(Phase::Turn, &before),
            report_for(Phase::Turn, &after)
        );
        assert_eq!(
            report_for(Phase::Turn, &after),
            expect(State::Working, None)
        );
    }

    #[test]
    fn a_rex_split_anchors_on_the_block_and_runs_the_command_interactively() {
        let block = RexBlock::parse("block:1").unwrap();
        let split = RexSplit {
            block: &block,
            label: "view a b.md",
            cwd: "/w d",
        };
        let command = ["ralph", "view", "/w d/a b.md"].map(str::to_string);
        assert_eq!(
            rex_split_argv(split, command.to_vec()),
            [
                "split",
                "-b",
                "block:1",
                "--label",
                "view a b.md",
                "-c",
                "/w d",
                "--interactive",
                "--",
                "ralph",
                "view",
                "/w d/a b.md",
            ]
        );
    }
}
