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
const RESUME_HINT: &str = "Turn it on again with /send-it.";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub max_turns: u32,
    pub max_usd: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Run {
    pub turns: u32,
    pub spent_usd: f64,
}

pub fn usage(defaults: &Limits) -> String {
    format!(
        "Usage: /send-it <instructions>. ralph restates them as numbered steps, each with an exit criterion, plus the actions the run may take; you approve once and the run needs no input. Default caps: {} turns, ${:.2}.",
        defaults.max_turns, defaults.max_usd
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    TurnCap,
    CostCap,
    Unmet { step: usize },
    WorkerFlagged,
    JudgeRounds { step: usize },
}

impl StopReason {
    pub fn notice(self, run: &Run) -> String {
        let why = match self {
            StopReason::TurnCap => format!("turn cap reached after {} turns", run.turns),
            StopReason::CostCap => format!("cost cap reached at ${:.2}", run.spent_usd),
            StopReason::Unmet { step } => format!("step {step} missed its exit criterion"),
            StopReason::WorkerFlagged => "a worker asked a question or is blocked".to_string(),
            StopReason::JudgeRounds { step } => {
                format!("step {step} was still not met after the last judge round")
            }
        };
        format!("send-it off: {why}. {RESUME_HINT}")
    }
}

fn flagged(report: &Report) -> bool {
    FLAGGED_STATUSES.contains(&report.status.as_str())
}

pub fn worker_flagged(unread: &[Report], drained: &[Report]) -> bool {
    unread.iter().chain(drained).any(flagged)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub step: usize,
    pub judge_rounds: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
    Start,
    TurnEnded(&'a Run),
    Report(&'a Report),
    Exited(i32),
    Verdict(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Send(String),
    Hold(String),
    Watch(String),
    Check(String),
    Judge,
    Wait,
    Stop(StopReason),
    Done,
}

fn granted_limits(plan: &RunPlan) -> Limits {
    plan.grants
        .iter()
        .fold(plan.limits, |limits, grant| match grant {
            Grant::TurnCap { turns } => Limits {
                max_turns: *turns,
                ..limits
            },
            Grant::CostCap { usd } => Limits {
                max_usd: *usd,
                ..limits
            },
        })
}

fn step_line(plan: &RunPlan, index: usize, step: &Step) -> String {
    format!(
        "[send-it {}/{}] {} · exit: {}",
        index + 1,
        plan.steps.len(),
        step.task,
        step.criterion.label()
    )
}

fn wait_on(criterion: &Criterion) -> Next {
    match criterion {
        Criterion::WorkerDone { worker } => Next::Hold(worker.clone()),
        Criterion::WatchPasses { command } => Next::Watch(command.clone()),
        Criterion::CheckPasses { command } => Next::Check(command.clone()),
        Criterion::Judge { .. } => Next::Judge,
    }
}

fn step_met(plan: &RunPlan, progress: Progress) -> (Progress, Next) {
    let index = progress.step + 1;
    match plan.steps.get(index) {
        Some(step) => (
            Progress {
                step: index,
                judge_rounds: 0,
            },
            Next::Send(step_line(plan, index, step)),
        ),
        None => (progress, Next::Done),
    }
}

pub fn advance(plan: &RunPlan, progress: Progress, event: Event) -> (Progress, Next) {
    let Some(step) = plan.steps.get(progress.step) else {
        return (progress, Next::Done);
    };
    let number = progress.step + 1;
    let unmet = (progress, Next::Stop(StopReason::Unmet { step: number }));
    match (event, &step.criterion) {
        (Event::Start, _) => (progress, Next::Send(step_line(plan, progress.step, step))),
        (Event::TurnEnded(run), criterion) => {
            let limits = granted_limits(plan);
            let next = if run.turns >= limits.max_turns {
                Next::Stop(StopReason::TurnCap)
            } else if run.spent_usd >= limits.max_usd {
                Next::Stop(StopReason::CostCap)
            } else {
                wait_on(criterion)
            };
            (progress, next)
        }
        (Event::Report(report), _) if flagged(report) => {
            (progress, Next::Stop(StopReason::WorkerFlagged))
        }
        (Event::Report(report), Criterion::WorkerDone { worker }) if report.worker == *worker => {
            if report.status == "done" {
                step_met(plan, progress)
            } else {
                unmet
            }
        }
        (Event::Exited(code), Criterion::WatchPasses { .. } | Criterion::CheckPasses { .. }) => {
            if code == 0 {
                step_met(plan, progress)
            } else {
                unmet
            }
        }
        (Event::Verdict(true), Criterion::Judge { .. }) => step_met(plan, progress),
        (Event::Verdict(false), Criterion::Judge { .. }) => {
            let judge_rounds = progress.judge_rounds + 1;
            if judge_rounds >= plan.judge_rounds {
                (
                    progress,
                    Next::Stop(StopReason::JudgeRounds { step: number }),
                )
            } else {
                (
                    Progress {
                        judge_rounds,
                        ..progress
                    },
                    Next::Send(step_line(plan, progress.step, step)),
                )
            }
        }
        _ => (progress, Next::Wait),
    }
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
