use crate::worker::Report;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Duration;

pub const WATCH_STATUS: &str = "watch";
pub const WATCH_ID_PREFIX: &str = "watch:";
pub const TAIL_MAX_LINES: usize = 50;
pub const TAIL_MAX_BYTES: usize = 8 * 1024;
const TRUNCATED_MARKER: &str = "[earlier output truncated]";
const NO_OUTPUT: &str = "(no output)";

pub fn watch_id(raw: &str) -> String {
    if raw.starts_with(WATCH_ID_PREFIX) {
        raw.to_string()
    } else {
        format!("{WATCH_ID_PREFIX}{raw}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchKind {
    Normal,
    Persistent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchSpec {
    pub id: String,
    pub command: String,
    pub follow_up: Option<String>,
    pub kind: WatchKind,
    pub timeout: Option<Duration>,
    pub limit: Option<Duration>,
}

impl WatchSpec {
    pub fn with_default_timeout(self, default: Duration) -> Self {
        if self.timeout.is_some() {
            return self;
        }
        Self {
            timeout: Some(default),
            limit: Some(default),
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchOutcome {
    Exited(i32),
    Signaled(i32),
    Stopped,
    TimedOut(Duration),
    WaitFailed(String),
}

impl WatchOutcome {
    pub fn from_exit(code: Option<i32>, signal: Option<i32>) -> Self {
        match (code, signal) {
            (Some(code), _) => Self::Exited(code),
            (None, Some(signal)) => Self::Signaled(signal),
            (None, None) => Self::WaitFailed("no exit code or signal".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildState {
    Running,
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    WaitFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollDecision {
    Wait,
    Finish(Option<WatchOutcome>),
}

pub fn poll_decision(
    spec: &WatchSpec,
    elapsed: Duration,
    stop: Option<StopReason>,
    child: ChildState,
) -> PollDecision {
    if let Some(reason) = stop {
        return PollDecision::Finish(reason.outcome());
    }
    if let Some(timeout) = spec.timeout.filter(|timeout| elapsed >= *timeout) {
        let limit = spec.limit.unwrap_or(timeout);
        return PollDecision::Finish(Some(WatchOutcome::TimedOut(limit)));
    }
    match child {
        ChildState::Running => PollDecision::Wait,
        ChildState::Exited { code, signal } => {
            PollDecision::Finish(Some(WatchOutcome::from_exit(code, signal)))
        }
        ChildState::WaitFailed(error) => {
            PollDecision::Finish(Some(WatchOutcome::WaitFailed(error)))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Requested,
    Shutdown,
}

impl StopReason {
    pub fn outcome(self) -> Option<WatchOutcome> {
        match self {
            Self::Requested => Some(WatchOutcome::Stopped),
            Self::Shutdown => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tail {
    max_lines: usize,
    max_bytes: usize,
    lines: VecDeque<String>,
    bytes: usize,
    truncated: bool,
}

impl Tail {
    pub fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            max_lines,
            max_bytes,
            lines: VecDeque::new(),
            bytes: 0,
            truncated: false,
        }
    }

    pub fn push_line(&mut self, line: &str) {
        let kept = clip_end(line, self.max_bytes);
        self.truncated |= kept.len() < line.len();
        self.bytes += kept.len();
        self.lines.push_back(kept.to_string());
        while self.lines.len() > self.max_lines || self.bytes > self.max_bytes {
            let Some(dropped) = self.lines.pop_front() else {
                break;
            };
            self.bytes -= dropped.len();
            self.truncated = true;
        }
    }

    pub fn render(&self) -> String {
        let body = self.lines.iter().map(String::as_str).collect::<Vec<_>>();
        match (self.truncated, body.is_empty()) {
            (true, true) => TRUNCATED_MARKER.to_string(),
            (true, false) => format!("{TRUNCATED_MARKER}\n{}", body.join("\n")),
            (false, _) => body.join("\n"),
        }
    }
}

fn clip_end(line: &str, max_bytes: usize) -> &str {
    let mut start = line.len().saturating_sub(max_bytes);
    while !line.is_char_boundary(start) {
        start += 1;
    }
    &line[start..]
}

fn describe(outcome: &WatchOutcome) -> String {
    match outcome {
        WatchOutcome::Exited(code) => format!("exited with code {code}"),
        WatchOutcome::Signaled(signal) => format!("was killed by signal {signal}"),
        WatchOutcome::Stopped => "was stopped".to_string(),
        WatchOutcome::TimedOut(limit) => {
            format!("timed out after {}s and was killed", limit.as_secs())
        }
        WatchOutcome::WaitFailed(error) => format!("could not be waited on: {error}"),
    }
}

pub fn report_text(spec: &WatchSpec, outcome: &WatchOutcome, tail: &Tail) -> String {
    let rendered = tail.render();
    let output = if rendered.is_empty() {
        NO_OUTPUT.to_string()
    } else {
        rendered
    };
    let mut text = format!(
        "Watch `{}` {}.\nCommand: {}\n\nOutput:\n{output}",
        spec.id,
        describe(outcome),
        spec.command
    );
    if let Some(follow_up) = &spec.follow_up {
        text.push_str(&format!("\n\nFollow-up: {follow_up}"));
    }
    text
}

pub fn build_report(spec: &WatchSpec, outcome: &WatchOutcome, tail: &Tail, at: &str) -> Report {
    Report {
        worker: spec.id.clone(),
        at: at.to_string(),
        status: WATCH_STATUS.to_string(),
        text: report_text(spec, outcome, tail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(follow_up: Option<&str>) -> WatchSpec {
        WatchSpec {
            id: "ci".to_string(),
            command: "gh run watch 1".to_string(),
            follow_up: follow_up.map(str::to_string),
            kind: WatchKind::Normal,
            timeout: None,
            limit: None,
        }
    }

    fn tail_of(lines: &[&str]) -> Tail {
        let mut tail = Tail::new(TAIL_MAX_LINES, TAIL_MAX_BYTES);
        lines.iter().for_each(|line| tail.push_line(line));
        tail
    }

    fn timed(timeout: u64, limit: u64) -> WatchSpec {
        let mut spec = spec(None);
        spec.timeout = Some(Duration::from_secs(timeout));
        spec.limit = Some(Duration::from_secs(limit));
        spec
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn poll_waits_while_running_before_the_timeout() {
        assert_eq!(
            poll_decision(&timed(60, 60), secs(59), None, ChildState::Running),
            PollDecision::Wait
        );
        assert_eq!(
            poll_decision(&spec(None), secs(99_999), None, ChildState::Running),
            PollDecision::Wait
        );
    }

    #[test]
    fn poll_times_out_at_the_timeout_and_reports_the_original_limit() {
        assert_eq!(
            poll_decision(&timed(10, 60), secs(10), None, ChildState::Running),
            PollDecision::Finish(Some(WatchOutcome::TimedOut(secs(60))))
        );
        assert_eq!(
            poll_decision(&timed(60, 60), secs(61), None, ChildState::Running),
            PollDecision::Finish(Some(WatchOutcome::TimedOut(secs(60))))
        );
    }

    #[test]
    fn poll_stop_wins_over_timeout_and_exit() {
        let exited = ChildState::Exited {
            code: Some(0),
            signal: None,
        };
        assert_eq!(
            poll_decision(
                &timed(1, 1),
                secs(5),
                Some(StopReason::Requested),
                exited.clone()
            ),
            PollDecision::Finish(Some(WatchOutcome::Stopped))
        );
        assert_eq!(
            poll_decision(&spec(None), secs(0), Some(StopReason::Shutdown), exited),
            PollDecision::Finish(None)
        );
    }

    #[test]
    fn poll_timeout_wins_over_exit() {
        let exited = ChildState::Exited {
            code: Some(0),
            signal: None,
        };
        assert_eq!(
            poll_decision(&timed(5, 5), secs(5), None, exited),
            PollDecision::Finish(Some(WatchOutcome::TimedOut(secs(5))))
        );
    }

    #[test]
    fn poll_reports_exit_and_wait_failure() {
        assert_eq!(
            poll_decision(
                &spec(None),
                secs(1),
                None,
                ChildState::Exited {
                    code: None,
                    signal: Some(9)
                }
            ),
            PollDecision::Finish(Some(WatchOutcome::Signaled(9)))
        );
        assert_eq!(
            poll_decision(
                &spec(None),
                secs(1),
                None,
                ChildState::WaitFailed("x".into())
            ),
            PollDecision::Finish(Some(WatchOutcome::WaitFailed("x".into())))
        );
    }

    #[test]
    fn watch_ids_get_the_prefix_once() {
        assert_eq!(watch_id("w1"), "watch:w1");
        assert_eq!(watch_id("watch:w1"), "watch:w1");
        assert_eq!(watch_id("foo"), "watch:foo");
    }

    #[test]
    fn a_watch_id_is_never_a_valid_worker_id() {
        for raw in ["w1", "ci", "watch:w1", "a-b"] {
            let args = serde_json::json!({"id": watch_id(raw), "text": "t"});
            assert!(matches!(
                crate::worker::parse_command("reply", &args),
                Some(Err(_))
            ));
        }
    }

    #[test]
    fn a_spec_without_a_timeout_gets_the_default_as_timeout_and_limit() {
        let spec = spec(None).with_default_timeout(Duration::from_secs(3600));
        assert_eq!(spec.timeout, Some(Duration::from_secs(3600)));
        assert_eq!(spec.limit, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn a_spec_with_a_timeout_keeps_it_and_its_limit() {
        let mut own = spec(None);
        own.timeout = Some(Duration::from_secs(10));
        own.limit = Some(Duration::from_secs(30));
        let kept = own.clone().with_default_timeout(Duration::from_secs(3600));
        assert_eq!(kept, own);
    }

    #[test]
    fn tail_under_the_limit_keeps_everything() {
        let tail = tail_of(&["a", "b"]);
        assert_eq!(tail.render(), "a\nb");
    }

    #[test]
    fn tail_over_the_line_limit_keeps_the_last_lines_and_marks_truncation() {
        let mut tail = Tail::new(2, 1024);
        ["a", "b", "c"].iter().for_each(|line| tail.push_line(line));
        assert_eq!(tail.render(), "[earlier output truncated]\nb\nc");
    }

    #[test]
    fn tail_over_the_byte_limit_drops_old_lines() {
        let mut tail = Tail::new(10, 4);
        ["aa", "bb", "cc"]
            .iter()
            .for_each(|line| tail.push_line(line));
        assert_eq!(tail.render(), "[earlier output truncated]\nbb\ncc");
    }

    #[test]
    fn tail_clips_one_long_line_on_a_char_boundary() {
        let mut tail = Tail::new(10, 3);
        tail.push_line("a\u{e9}\u{e9}\u{e9}");
        assert_eq!(tail.render(), "[earlier output truncated]\n\u{e9}");
    }

    #[test]
    fn empty_tail_renders_empty() {
        assert_eq!(tail_of(&[]).render(), "");
    }

    #[test]
    fn zero_capacity_tail_renders_only_the_marker() {
        let mut tail = Tail::new(0, 0);
        tail.push_line("x");
        assert_eq!(tail.render(), "[earlier output truncated]");
    }

    #[test]
    fn exit_maps_code_then_signal() {
        assert_eq!(
            WatchOutcome::from_exit(Some(3), None),
            WatchOutcome::Exited(3)
        );
        assert_eq!(
            WatchOutcome::from_exit(None, Some(9)),
            WatchOutcome::Signaled(9)
        );
        assert!(matches!(
            WatchOutcome::from_exit(None, None),
            WatchOutcome::WaitFailed(_)
        ));
    }

    #[test]
    fn only_a_requested_stop_has_an_outcome() {
        assert_eq!(StopReason::Requested.outcome(), Some(WatchOutcome::Stopped));
        assert_eq!(StopReason::Shutdown.outcome(), None);
    }

    #[test]
    fn report_for_exit_zero() {
        let text = report_text(&spec(None), &WatchOutcome::Exited(0), &tail_of(&["ok"]));
        assert!(text.starts_with("Watch `ci` exited with code 0."));
        assert!(text.contains("Command: gh run watch 1"));
        assert!(text.ends_with("Output:\nok"));
    }

    #[test]
    fn report_for_nonzero_exit() {
        let text = report_text(&spec(None), &WatchOutcome::Exited(3), &tail_of(&["bad"]));
        assert!(text.contains("exited with code 3."));
    }

    #[test]
    fn report_for_killed() {
        let text = report_text(&spec(None), &WatchOutcome::Signaled(9), &tail_of(&[]));
        assert!(text.contains("was killed by signal 9."));
        assert!(text.ends_with("(no output)"));
        let stopped = report_text(&spec(None), &WatchOutcome::Stopped, &tail_of(&[]));
        assert!(stopped.contains("was stopped."));
    }

    #[test]
    fn report_for_timeout() {
        let outcome = WatchOutcome::TimedOut(Duration::from_secs(30));
        let text = report_text(&spec(None), &outcome, &tail_of(&[]));
        assert!(text.contains("timed out after 30s and was killed."));
    }

    #[test]
    fn report_includes_the_follow_up_when_present() {
        let text = report_text(
            &spec(Some("Merge the PR.")),
            &WatchOutcome::Exited(0),
            &tail_of(&["ok"]),
        );
        assert!(text.ends_with("\n\nFollow-up: Merge the PR."));
        let without = report_text(&spec(None), &WatchOutcome::Exited(0), &tail_of(&["ok"]));
        assert!(!without.contains("Follow-up"));
    }

    #[test]
    fn build_report_uses_the_watch_status_and_given_time() {
        let report = build_report(&spec(None), &WatchOutcome::Exited(0), &tail_of(&[]), "t1");
        assert_eq!(report.worker, "ci");
        assert_eq!(report.at, "t1");
        assert_eq!(report.status, WATCH_STATUS);
        assert!(report.text.contains("exited with code 0"));
    }

    #[test]
    fn a_line_of_exactly_the_byte_limit_is_not_truncated() {
        let line = "a".repeat(8192);
        assert_eq!(tail_of(&[&line]).render(), line);
        let longer = "a".repeat(8193);
        assert!(tail_of(&[&longer]).render().starts_with(TRUNCATED_MARKER));
    }
}
