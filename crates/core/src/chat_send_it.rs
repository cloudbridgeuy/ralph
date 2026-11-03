use crate::inbox_alert::{Alert, AlertConfig};
use crate::program_status::{Kind, Report as ProgramReport, State};
use crate::worker::{HarnessId, Report};
use serde::Deserialize;
use serde_json::Value;

pub const DEFAULT_JUDGE_ROUNDS: u32 = 2;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Criterion {
    WorkerDone { worker: String },
    WatchPasses { command: String },
    CheckPasses { command: String },
    Judge { question: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Spawn { harness: HarnessId },
    Reply,
    Close,
    Watch,
    Merge,
    Push,
    Pr,
    LinearWrite,
    PaidRun { max_usd: f64 },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Grant {
    TurnCap { turns: u32 },
    CostCap { usd: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub task: String,
    pub criterion: Criterion,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunPlan {
    pub goal: String,
    pub steps: Vec<Step>,
    pub allowed: Vec<Action>,
    pub grants: Vec<Grant>,
    pub limits: Limits,
    pub judge_rounds: u32,
}

#[derive(Deserialize)]
struct RawPlan {
    goal: String,
    steps: Vec<RawStep>,
    #[serde(default)]
    allowed: Vec<Action>,
    #[serde(default)]
    grants: Vec<Grant>,
    #[serde(default)]
    caps: RawCaps,
}

#[derive(Deserialize)]
struct RawStep {
    #[serde(rename = "do")]
    task: String,
    criterion: Option<Criterion>,
}

#[derive(Deserialize, Default)]
struct RawCaps {
    turns: Option<u32>,
    usd: Option<f64>,
    judge_rounds: Option<u32>,
}

fn parse_step(number: usize, step: RawStep) -> Result<Step, String> {
    let task = step.task.trim();
    if task.is_empty() {
        return Err(format!("Step {number} is empty. Say what the step does."));
    }
    let criterion = step.criterion.ok_or_else(|| {
        format!("Step {number} has no exit criterion. Give each step a check ralph can measure.")
    })?;
    Ok(Step {
        task: task.to_string(),
        criterion,
    })
}

pub fn parse_run(args: &Value, defaults: Limits) -> Result<RunPlan, String> {
    let raw =
        RawPlan::deserialize(args).map_err(|error| format!("Run plan is malformed: {error}."))?;
    let goal = raw.goal.trim();
    if goal.is_empty() {
        return Err("Run plan has no goal. Say what the run delivers.".to_string());
    }
    if raw.steps.is_empty() {
        return Err("Run plan has no steps. List the steps in order.".to_string());
    }
    let steps = raw
        .steps
        .into_iter()
        .enumerate()
        .map(|(index, step)| parse_step(index + 1, step))
        .collect::<Result<Vec<_>, _>>()?;
    let limits = Limits {
        max_turns: raw.caps.turns.unwrap_or(defaults.max_turns),
        max_usd: raw.caps.usd.unwrap_or(defaults.max_usd),
    };
    let judge_rounds = raw.caps.judge_rounds.unwrap_or(DEFAULT_JUDGE_ROUNDS);
    if limits.max_turns == 0
        || judge_rounds == 0
        || limits.max_usd.is_nan()
        || limits.max_usd <= 0.0
    {
        return Err("Run plan caps must be positive.".to_string());
    }
    Ok(RunPlan {
        goal: goal.to_string(),
        steps,
        allowed: raw.allowed,
        grants: raw.grants,
        limits,
        judge_rounds,
    })
}

impl Criterion {
    fn label(&self) -> String {
        match self {
            Criterion::WorkerDone { worker } => format!("worker {worker} reports done"),
            Criterion::WatchPasses { command } => format!("watch `{command}` exits 0"),
            Criterion::CheckPasses { command } => format!("`{command}` exits 0"),
            Criterion::Judge { question } => format!("judged: {question}"),
        }
    }
}

impl Action {
    fn label(&self) -> String {
        match self {
            Action::Spawn { harness } => format!("spawn {} workers", harness.as_str()),
            Action::Reply => "reply to workers".to_string(),
            Action::Close => "close workers".to_string(),
            Action::Watch => "start watches".to_string(),
            Action::Merge => "merge".to_string(),
            Action::Push => "push".to_string(),
            Action::Pr => "open PRs".to_string(),
            Action::LinearWrite => "write to Linear".to_string(),
            Action::PaidRun { max_usd } => format!("paid runs up to ${max_usd:.2}"),
        }
    }
}

impl Grant {
    fn label(&self) -> String {
        match self {
            Grant::TurnCap { turns } => format!("raise the turn cap to {turns}"),
            Grant::CostCap { usd } => format!("raise the cost cap to ${usd:.2}"),
        }
    }
}

pub fn render_run(plan: &RunPlan) -> String {
    let mut lines = vec![format!("Goal: {}", plan.goal), "Steps:".to_string()];
    for (index, step) in plan.steps.iter().enumerate() {
        lines.push(format!("  {}. {}", index + 1, step.task));
        lines.push(format!("     exit: {}", step.criterion.label()));
    }
    let allowed: Vec<String> = plan.allowed.iter().map(Action::label).collect();
    lines.push(if allowed.is_empty() {
        "Allowed: none".to_string()
    } else {
        format!("Allowed: {}", allowed.join(", "))
    });
    lines.push(format!(
        "Caps: {} turns, ${:.2}, {} judge rounds",
        plan.limits.max_turns, plan.limits.max_usd, plan.judge_rounds
    ));
    lines.push("You pre-approve:".to_string());
    if plan.grants.is_empty() {
        lines.push("  - nothing".to_string());
    }
    lines.extend(
        plan.grants
            .iter()
            .map(|grant| format!("  - {}", grant.label())),
    );
    lines.join("\n")
}

const FLAGGED_STATUSES: [&str; 2] = ["question", "blocked"];
const COUNTDOWN_PREFIX: &str = "send-it in ";
const COUNTDOWN_HINT: &str = "  (any key cancels)";
pub const CLEAR_LINE: &str = "\r\x1b[2K";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const AUTO_PREFIX: &str = "[send-it] ";
const RESUME_HINT: &str = "Turn it on again with /send-it.";

const DECISION_TOKENS: [&str; 64] = [
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
    "done",
    "mark",
    "marked",
    "file",
    "filed",
    "cancel",
    "cancelled",
    "label",
    "labels",
    "comment",
    "comments",
    "close",
    "closed",
    "closes",
    "release",
    "tag",
    "tags",
    "reset",
    "force",
    "drop",
    "dropping",
    "delete",
    "deleted",
    "kill",
    "killed",
    "plan",
    "planned",
    "proposed",
    "option",
    "pick",
    "ticket",
    "tickets",
    "linear",
    "haiku",
    "sonnet",
    "opus",
    "paid",
    "i",
    "my",
    "deploy",
    "publish",
    "rm",
    "revert",
    "squash",
];

const ROUTINE_TOKENS: [&str; 11] = [
    "fix",
    "fixes",
    "checks",
    "test",
    "tests",
    "reinstall",
    "rebase",
    "retry",
    "lint",
    "fmt",
    "clippy",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendIt {
    On,
    Off,
}

impl SendIt {
    pub fn parse(args: &str) -> Option<SendIt> {
        let mut tokens = args.split_whitespace();
        match (tokens.next(), tokens.next()) {
            (None, _) => Some(SendIt::On),
            (Some(word), None) if word.eq_ignore_ascii_case("off") => Some(SendIt::Off),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub max_turns: u32,
    pub max_usd: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Run {
    pub turns: u32,
    pub spent_usd: f64,
    pub sent: Vec<String>,
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl Run {
    pub fn record(&mut self, text: &str, cost_usd: Option<f64>) {
        self.turns += 1;
        self.spent_usd += cost_usd.unwrap_or(0.0);
        self.sent.push(normalize(text));
    }

    fn already_sent(&self, text: &str) -> bool {
        self.sent.contains(&normalize(text))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    pub text: &'a str,
    pub worker_flagged: bool,
    pub schedule_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualReason {
    SlashCommand,
    Decision(&'static str),
    NotRoutine,
    WorkerFlagged,
    SchedulePending,
}

impl ManualReason {
    pub fn notice(self) -> String {
        let why = match self {
            ManualReason::SlashCommand => "a slash command".to_string(),
            ManualReason::Decision(word) => format!("a decision ({word})"),
            ManualReason::NotRoutine => "no routine action".to_string(),
            ManualReason::WorkerFlagged => "a worker question or block".to_string(),
            ManualReason::SchedulePending => "a pending schedule".to_string(),
        };
        format!("send-it waits on you: {why}.")
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
        format!("send-it off: {why}. {RESUME_HINT}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Countdown,
    Manual(ManualReason),
    Stop(StopReason),
}

pub fn text_reason(text: &str) -> Option<ManualReason> {
    let lowered = text.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    if let Some(word) = tokens
        .iter()
        .find_map(|token| DECISION_TOKENS.iter().find(|word| *word == token))
    {
        return Some(ManualReason::Decision(word));
    }
    if tokens
        .windows(2)
        .any(|pair| pair[0] == "pull" && matches!(pair[1], "request" | "requests"))
    {
        return Some(ManualReason::Decision("pull request"));
    }
    (!tokens.iter().any(|token| ROUTINE_TOKENS.contains(token))).then_some(ManualReason::NotRoutine)
}

pub fn decide(run: &Run, limits: &Limits, facts: &Facts) -> Decision {
    if facts.text.starts_with('/') {
        return Decision::Manual(ManualReason::SlashCommand);
    }
    let reason = text_reason(facts.text);
    if let Some(word @ ManualReason::Decision(_)) = reason {
        return Decision::Manual(word);
    }
    if facts.worker_flagged {
        return Decision::Manual(ManualReason::WorkerFlagged);
    }
    if facts.schedule_pending {
        return Decision::Manual(ManualReason::SchedulePending);
    }
    if let Some(reason) = reason {
        return Decision::Manual(reason);
    }
    if run.turns >= limits.max_turns {
        Decision::Stop(StopReason::TurnCap)
    } else if run.spent_usd >= limits.max_usd {
        Decision::Stop(StopReason::CostCap)
    } else if run.already_sent(facts.text) {
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
        None => "send-it is off. Turn it on with /send-it.".to_string(),
        Some(run) => format!(
            "send-it on: {} of {} turns, ${:.2} of ${:.2}. Countdown: {seconds}s.",
            run.turns, limits.max_turns, run.spent_usd, limits.max_usd
        ),
    }
}

pub fn cancel_notice() -> String {
    format!("send-it cancelled by a key press. {RESUME_HINT}")
}

pub fn no_terminal_notice() -> String {
    format!("send-it off: the countdown needs a terminal. {RESUME_HINT}")
}

pub fn blocked_alert(notice: &str, config: &AlertConfig) -> Option<Alert> {
    (config.enabled() && !config.terminal_status).then(|| Alert {
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
