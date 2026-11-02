use crate::chat_hold::hold_targets;
use crate::chat_status::Status;
use crate::inbox_alert::{Alert, AlertConfig};
use crate::program_status::{Kind, Report as ProgramReport, State};
use crate::worker::Report;

const FLAGGED_STATUSES: [&str; 2] = ["question", "blocked"];
const COUNTDOWN_PREFIX: &str = "send-it in ";
const COUNTDOWN_HINT: &str = "  (any key cancels)";
pub const CLEAR_LINE: &str = "\r\x1b[2K";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const AUTO_PREFIX: &str = "[send-it] ";

const VETO_TOKENS: [&str; 20] = [
    "push",
    "pushed",
    "pushes",
    "pushing",
    "merge",
    "merged",
    "merges",
    "merging",
    "pr",
    "prs",
    "abandon",
    "abandoned",
    "abandons",
    "abandoning",
    "approve",
    "approved",
    "approves",
    "approving",
    "approval",
    "lgtm",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    FixRound,
    LiveCheck,
    Checks,
    Reinstall,
    DoneCleanup,
}

impl Step {
    pub const ALL: [Step; 5] = [
        Step::FixRound,
        Step::LiveCheck,
        Step::Checks,
        Step::Reinstall,
        Step::DoneCleanup,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Step::FixRound => "fix_round",
            Step::LiveCheck => "live_check",
            Step::Checks => "checks",
            Step::Reinstall => "reinstall",
            Step::DoneCleanup => "done_cleanup",
        }
    }

    pub fn parse(value: &str) -> Option<Step> {
        Step::ALL.into_iter().find(|step| step.as_str() == value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendIt {
    Arm(String),
    Off,
    Status,
}

impl SendIt {
    pub fn parse(args: &str) -> Option<SendIt> {
        let mut tokens = args.split_whitespace();
        match (tokens.next(), tokens.next()) {
            (None, _) => Some(SendIt::Status),
            (Some("off"), None) => Some(SendIt::Off),
            (Some(ticket), None) => Some(SendIt::Arm(ticket.to_string())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub max_turns: u32,
    pub max_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub ticket: String,
    pub turns: u32,
    pub spent_usd: f64,
    pub sent: Vec<(Step, String)>,
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl Run {
    pub fn new(ticket: &str) -> Self {
        Self {
            ticket: ticket.to_string(),
            turns: 0,
            spent_usd: 0.0,
            sent: Vec::new(),
        }
    }

    pub fn record(&mut self, step: Step, text: &str, cost_usd: Option<f64>) {
        self.turns += 1;
        self.spent_usd += cost_usd.unwrap_or(0.0);
        self.sent.push((step, normalize(text)));
    }

    fn resume_hint(&self) -> String {
        format!("Resume with /send-it {}", self.ticket)
    }

    fn already_sent(&self, step: Step, text: &str) -> bool {
        let text = normalize(text);
        self.sent
            .iter()
            .any(|(sent_step, sent_text)| *sent_step == step && *sent_text == text)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    pub text: &'a str,
    pub step: Option<Step>,
    pub worker_flagged: bool,
    pub schedule_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualReason {
    NoStep,
    SlashCommand,
    VetoWord,
    WorkerFlagged,
    SchedulePending,
}

impl ManualReason {
    pub fn notice(self, run: &Run) -> String {
        let why = match self {
            ManualReason::NoStep => "this reply is not a routine step",
            ManualReason::SlashCommand => "this reply is a slash command",
            ManualReason::VetoWord => "this reply mentions push, merge, PR, approve or abandon",
            ManualReason::WorkerFlagged => "a worker needs attention",
            ManualReason::SchedulePending => "a scheduled task is pending",
        };
        format!("send-it paused: {why}. {}", run.resume_hint())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    TurnCap,
    CostCap,
    Repeated,
}

impl StopReason {
    pub fn notice(self, run: &Run) -> String {
        let why = match self {
            StopReason::TurnCap => format!("turn cap reached after {} turns", run.turns),
            StopReason::CostCap => format!("cost cap reached at ${:.2}", run.spent_usd),
            StopReason::Repeated => "the same reply came up again".to_string(),
        };
        format!("send-it stopped: {why}. {}", run.resume_hint())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Countdown,
    Manual(ManualReason),
    Stop(StopReason),
}

pub fn vetoed_text(text: &str) -> bool {
    let lowered = text.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    tokens.iter().any(|token| VETO_TOKENS.contains(token))
        || tokens
            .windows(2)
            .any(|pair| pair[0] == "pull" && matches!(pair[1], "request" | "requests"))
}

pub fn decide(run: &Run, limits: &Limits, facts: &Facts) -> Decision {
    if facts.text.starts_with('/') {
        return Decision::Manual(ManualReason::SlashCommand);
    }
    if vetoed_text(facts.text) {
        return Decision::Manual(ManualReason::VetoWord);
    }
    if facts.worker_flagged {
        return Decision::Manual(ManualReason::WorkerFlagged);
    }
    if facts.schedule_pending {
        return Decision::Manual(ManualReason::SchedulePending);
    }
    let Some(step) = facts.step else {
        return Decision::Manual(ManualReason::NoStep);
    };
    if run.turns >= limits.max_turns {
        Decision::Stop(StopReason::TurnCap)
    } else if run.spent_usd >= limits.max_usd {
        Decision::Stop(StopReason::CostCap)
    } else if run.already_sent(step, facts.text) {
        Decision::Stop(StopReason::Repeated)
    } else {
        Decision::Countdown
    }
}

pub fn worker_flagged(unread: &[Report], drained: &[Report]) -> bool {
    unread
        .iter()
        .chain(drained)
        .any(|report| FLAGGED_STATUSES.contains(&report.status.as_str()))
}

pub fn remaining_seconds(elapsed_ms: u64, total_seconds: u64) -> u64 {
    (total_seconds * 1_000)
        .saturating_sub(elapsed_ms)
        .div_ceil(1_000)
}

pub fn countdown_frame(seconds_left: u64, text: &str, columns: usize, color: bool) -> String {
    let head = format!("{COUNTDOWN_PREFIX}{seconds_left}s: ");
    let budget = columns.saturating_sub(head.chars().count() + COUNTDOWN_HINT.chars().count() + 1);
    let shown = if text.chars().count() <= budget {
        text.to_string()
    } else {
        let kept: String = text.chars().take(budget.saturating_sub(1)).collect();
        format!("{kept}…")
    };
    let line = format!("{head}{shown}{COUNTDOWN_HINT}");
    if color {
        format!("{CLEAR_LINE}{DIM}{line}{RESET}")
    } else {
        format!("{CLEAR_LINE}{line}")
    }
}

pub fn auto_line(text: &str) -> String {
    format!("{AUTO_PREFIX}{text}")
}

pub fn status_text(run: Option<&Run>, limits: &Limits, seconds: u64) -> String {
    match run {
        None => "send-it is off. Arm it with /send-it <ticket>.".to_string(),
        Some(run) => format!(
            "send-it armed for {}: {} of {} turns, ${:.2} of ${:.2}. Countdown: {seconds}s.",
            run.ticket, run.turns, limits.max_turns, run.spent_usd, limits.max_usd
        ),
    }
}

pub fn cancel_notice(run: &Run) -> String {
    format!("send-it cancelled by a key press. {}", run.resume_hint())
}

pub fn no_terminal_notice(run: &Run) -> String {
    format!(
        "send-it paused: the countdown needs a terminal. {}",
        run.resume_hint()
    )
}

pub fn closed_notice(run: &Run) -> String {
    format!("send-it off: worker {} is no longer listed.", run.ticket)
}

pub fn tracks_worker(ticket: &str, status: &Status) -> bool {
    hold_targets(status).contains(ticket)
}

pub fn worker_closed(tracked: bool, ticket: &str, status: &Status) -> bool {
    tracked && !tracks_worker(ticket, status)
}

pub fn blocked_alert(notice: &str, config: &AlertConfig) -> Option<Alert> {
    config.enabled().then(|| Alert {
        sound: config.sound,
        notification: config.notification.then(|| notice.to_string()),
    })
}

pub fn blocked_report(notice: &str) -> ProgramReport {
    ProgramReport {
        state: State::Blocked(Kind::Question),
        msg: Some(notice.to_string()),
    }
}

#[cfg(test)]
mod tests;
