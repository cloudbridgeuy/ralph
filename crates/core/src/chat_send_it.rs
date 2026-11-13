use crate::chat_help::{wrap, Command};
use crate::inbox_alert::{Alert, AlertConfig};
use crate::program_status::{Kind, Report as ProgramReport, State};
use crate::watch::{
    describe, watch_id, WatchKind, WatchOutcome, WatchSpec, NO_OUTPUT, OUTPUT_HEADER,
};
use crate::worker::{HarnessId, Report};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use unicode_width::UnicodeWidthStr;

mod drive;
pub mod template;

pub use drive::*;

pub const DEFAULT_JUDGE_ROUNDS: u32 = 2;
pub const STEP_RESULT: &str = "step_result";
pub const STEP_RESULT_MAX_CHARS: usize = 300;
const NO_STEP_RESULT: &str = "the judge turn made no step_result call";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepResult {
    pub met: bool,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Criterion {
    WorkerDone { worker: String },
    WatchPasses { command: String },
    CheckPasses { command: String },
    Judge { question: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
struct RawStep {
    #[serde(rename = "do")]
    task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    criterion: Option<Criterion>,
}

#[derive(Serialize, Deserialize, Default)]
struct RawCaps {
    #[serde(skip_serializing_if = "Option::is_none")]
    turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    judge_rounds: Option<u32>,
}

impl From<&RunPlan> for RawPlan {
    fn from(plan: &RunPlan) -> Self {
        RawPlan {
            goal: plan.goal.clone(),
            steps: plan
                .steps
                .iter()
                .map(|step| RawStep {
                    task: step.task.clone(),
                    criterion: Some(step.criterion.clone()),
                })
                .collect(),
            allowed: plan.allowed.clone(),
            grants: plan.grants.clone(),
            caps: RawCaps {
                turns: Some(plan.limits.max_turns),
                usd: Some(plan.limits.max_usd),
                judge_rounds: Some(plan.judge_rounds),
            },
        }
    }
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

const RUN_BY_RALPH_NOTE: &str = "ralph runs this check alone: put any work in an earlier step";
const RUN_BY_RALPH_RULE: &str = "A watch_passes or check_passes step is run by ralph alone, with no model turn: put any work it needs in an earlier step.";

impl Criterion {
    fn runs_in_ralph(&self) -> bool {
        matches!(
            self,
            Criterion::WatchPasses { .. } | Criterion::CheckPasses { .. }
        )
    }

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
        if step.criterion.runs_in_ralph() {
            lines.push(format!("     {RUN_BY_RALPH_NOTE}"));
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendItCommand {
    Usage,
    Propose(String),
    Edit,
    Go,
    Status,
    Off,
    Save {
        name: String,
        ticket: Option<String>,
    },
    Templates,
}

pub fn parse_send_it_args(args: &str) -> SendItCommand {
    match args.trim() {
        "" => SendItCommand::Usage,
        "edit" => SendItCommand::Edit,
        "go" => SendItCommand::Go,
        "status" => SendItCommand::Status,
        "off" => SendItCommand::Off,
        "templates" => SendItCommand::Templates,
        text => {
            let words: Vec<&str> = text.split_whitespace().collect();
            match words.as_slice() {
                ["save", name] | ["save", name, _] => SendItCommand::Save {
                    name: name.to_string(),
                    ticket: words.get(2).map(|ticket| ticket.to_string()),
                },
                _ => SendItCommand::Propose(text.to_string()),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunFile {
    pub plan: RunPlan,
    pub approved: Option<String>,
    pub stopped: Option<Stopped>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stopped {
    pub step: usize,
    pub hash: String,
}

const APPROVED_KEY: &str = "approved";
const STOPPED_KEY: &str = "stopped";
const HASH_BYTES: usize = 6;

pub fn plan_hash(plan: &RunPlan) -> String {
    let canonical = serde_json::to_string(&RawPlan::from(plan)).unwrap_or_default();
    Sha256::digest(canonical.as_bytes())
        .iter()
        .take(HASH_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl RunFile {
    pub fn is_approved(&self) -> bool {
        self.approved.as_deref() == Some(plan_hash(&self.plan).as_str())
    }
}

pub fn read_run(text: &str, defaults: Limits) -> Result<RunFile, String> {
    let malformed = |error: String| format!("run.toml is malformed: {}", error.trim());
    let table: toml::Table = toml::from_str(text).map_err(|e| malformed(e.to_string()))?;
    let value = serde_json::to_value(table).map_err(|e| malformed(e.to_string()))?;
    Ok(RunFile {
        plan: parse_run(&value, defaults)?,
        approved: value
            .get(APPROVED_KEY)
            .and_then(Value::as_str)
            .map(str::to_string),
        stopped: value
            .get(STOPPED_KEY)
            .and_then(|stopped| Stopped::deserialize(stopped).ok()),
    })
}

pub fn write_run(file: &RunFile) -> Result<String, String> {
    let unwritable = |error: String| format!("Could not write run.toml: {error}");
    let mut value =
        serde_json::to_value(RawPlan::from(&file.plan)).map_err(|e| unwritable(e.to_string()))?;
    if let (Some(fields), Some(hash)) = (value.as_object_mut(), &file.approved) {
        fields.insert(APPROVED_KEY.to_string(), Value::String(hash.clone()));
    }
    if let (Some(fields), Some(stopped)) = (value.as_object_mut(), &file.stopped) {
        fields.insert(STOPPED_KEY.to_string(), json!(stopped));
    }
    toml::to_string(&value).map_err(|e| unwritable(e.to_string()))
}

pub fn approve(
    file: &RunFile,
    shown: Option<&str>,
    typed_at_prompt: bool,
) -> Result<RunFile, String> {
    let hash = plan_hash(&file.plan);
    if !typed_at_prompt {
        return Err("/send-it go counts only when you type it at the prompt.".to_string());
    }
    if shown != Some(hash.as_str()) {
        return Err(format!(
            "Plan {hash} is not the plan on screen. Show it with /send-it status, then type /send-it go."
        ));
    }
    Ok(RunFile {
        approved: Some(hash),
        ..file.clone()
    })
}

pub fn stop_file(file: &RunFile, hash: &str, stopped_at: Option<usize>) -> Option<RunFile> {
    (file.approved.as_deref() == Some(hash)).then(|| RunFile {
        plan: file.plan.clone(),
        approved: None,
        stopped: stopped_at.map(|step| Stopped {
            step,
            hash: hash.to_string(),
        }),
    })
}

fn resume_step(file: &RunFile) -> Option<usize> {
    let hash = plan_hash(&file.plan);
    file.stopped
        .as_ref()
        .filter(|stopped| stopped.hash == hash)
        .map(|stopped| stopped.step)
        .filter(|step| (1..=file.plan.steps.len()).contains(step))
}

fn hang_wrapped(line: &str, width: usize) -> String {
    let body = line.trim_start();
    let lead = &line[..line.len() - body.len()];
    match body.split_once(' ') {
        Some((marker, text)) if line.width() > width => {
            let hang = lead.width() + marker.width() + 1;
            let pad = " ".repeat(hang);
            wrap(text, width.saturating_sub(hang))
                .iter()
                .enumerate()
                .map(|(index, part)| match index {
                    0 => format!("{lead}{marker} {part}"),
                    _ => format!("{pad}{part}"),
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => line.to_string(),
    }
}

pub fn render_status(file: &RunFile, width: usize) -> String {
    let hash = plan_hash(&file.plan);
    let state = if file.is_approved() {
        format!("Plan {hash}: approved.")
    } else if let Some(step) = resume_step(file) {
        format!(
            "Plan {hash}: stopped at step {step}/{}. Type /send-it go to resume from step {step}.",
            file.plan.steps.len()
        )
    } else {
        format!("Plan {hash}: not approved. Type /send-it go to approve this plan.")
    };
    format!("{}\n{state}", render_run(&file.plan))
        .lines()
        .map(|line| hang_wrapped(line, width))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn propose_refusal(existing: Option<&RunFile>) -> Option<String> {
    existing.filter(|file| file.is_approved()).map(|file| {
        format!(
            "Plan {} is approved and active. Stop it with /send-it off first.",
            plan_hash(&file.plan)
        )
    })
}

pub fn propose_prompt(text: &str) -> String {
    format!(
        "/send-it: restate these instructions as a run plan. Call `propose_run` once with the plan, then stop. Do not start the work: ralph shows the plan and the person approves it with /send-it go. {RUN_BY_RALPH_RULE}\n\n{text}"
    )
}

pub fn propose_run_tool() -> Value {
    json!({
        "name": "propose_run",
        "description": "Store the run plan for a /send-it request. Call it only for a /send-it request, once, and do not start the work. Give every step an exit criterion ralph can measure. ralph shows the plan; the person approves it by typing /send-it go.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "goal": {"type": "string", "description": "What the run delivers."},
                "steps": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "do": {"type": "string", "description": "What the step does."},
                            "criterion": {
                                "type": "object",
                                "description": format!("Exit check. worker_done needs worker; watch_passes and check_passes need a command that exits 0; judge needs a question. {RUN_BY_RALPH_RULE}"),
                                "properties": {
                                    "kind": {"type": "string", "enum": ["worker_done", "watch_passes", "check_passes", "judge"]},
                                    "worker": {"type": "string"},
                                    "command": {"type": "string"},
                                    "question": {"type": "string"}
                                },
                                "required": ["kind"]
                            }
                        },
                        "required": ["do", "criterion"]
                    }
                },
                "allowed": {
                    "type": "array",
                    "description": "Actions the run may take. spawn needs harness; paid_run needs max_usd.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": {"type": "string", "enum": ["spawn", "reply", "close", "watch", "merge", "push", "pr", "linear_write", "paid_run"]},
                            "harness": {"type": "string"},
                            "max_usd": {"type": "number"}
                        },
                        "required": ["kind"]
                    }
                },
                "grants": {
                    "type": "array",
                    "description": "Limits the person pre-approves. turn_cap needs turns; cost_cap needs usd.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": {"type": "string", "enum": ["turn_cap", "cost_cap"]},
                            "turns": {"type": "integer"},
                            "usd": {"type": "number"}
                        },
                        "required": ["kind"]
                    }
                },
                "caps": {
                    "type": "object",
                    "description": "Omit to take the configured defaults.",
                    "properties": {
                        "turns": {"type": "integer", "minimum": 1},
                        "usd": {"type": "number", "exclusiveMinimum": 0},
                        "judge_rounds": {"type": "integer", "minimum": 1}
                    }
                }
            },
            "required": ["goal", "steps"]
        }
    })
}

pub fn step_result_tool() -> Value {
    json!({
        "name": STEP_RESULT,
        "description": "Judge the current /send-it judge step. Call it once, only in a send-it judge turn, from what the chat already shows. ralph moves on when met is true and sends the step again with your evidence when it is false.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "met": {"type": "boolean", "description": "true only when the step question is answered yes."},
                "evidence": {"type": "string", "maxLength": STEP_RESULT_MAX_CHARS, "description": "One line: the fact that decides it, such as a commit, a test count or an open finding."}
            },
            "required": ["met", "evidence"]
        }
    })
}

pub fn parse_step_result(args: &Value) -> Result<StepResult, String> {
    let met = args
        .get("met")
        .and_then(Value::as_bool)
        .ok_or("Missing boolean argument 'met'.")?;
    let evidence = args
        .get("evidence")
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or("Missing string argument 'evidence'.")?;
    if evidence.is_empty() {
        Err("The evidence is empty. Give the fact that decides the step.".to_string())
    } else if evidence.chars().any(char::is_control) {
        Err("The evidence must be one line.".to_string())
    } else if evidence.chars().count() > STEP_RESULT_MAX_CHARS {
        Err(format!(
            "The evidence is longer than {STEP_RESULT_MAX_CHARS} characters."
        ))
    } else {
        Ok(StepResult {
            met,
            evidence: evidence.to_string(),
        })
    }
}

const FLAGGED_STATUSES: [&str; 2] = ["question", "blocked"];
const RESTART_HINT: &str =
    "To run it again, show the plan with /send-it status, then type /send-it go.";
const OFF_NOTICE: &str = "send-it off: the run stopped and its plan is deleted.";

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
        "Usage: /send-it <instructions>. ralph restates them as numbered steps, each with an exit criterion, plus the actions the run may take; you approve once and the run needs no input. Default caps: {} turns, ${:.2}. Templates: /send-it save <name> [<ticket>], /send-it templates, /send-it <ticket> <name>.",
        defaults.max_turns, defaults.max_usd
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    TurnCap,
    CostCap,
    Unmet { step: usize },
    WorkerFlagged,
    JudgeRounds { step: usize, evidence: String },
    CheckFailed { step: usize, detail: Option<String> },
    CheckNotStarted { step: usize, error: String },
    Off,
    PlanChanged,
    NoReply,
    NeedsYou,
    TookOver,
    Refused { tool: String },
}

const DETAIL_LINES: usize = 3;
const DETAIL_CHARS: usize = 200;
const ELLIPSIS: &str = "...";

fn output_tail(text: &str) -> Option<String> {
    let output = text.split_once(OUTPUT_HEADER)?.1;
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(DETAIL_LINES)..].join(" / ");
    let chars = tail.chars().count();
    if tail.is_empty() || tail == NO_OUTPUT {
        None
    } else if chars > DETAIL_CHARS {
        let keep = DETAIL_CHARS - ELLIPSIS.len();
        let kept: String = tail.chars().skip(chars - keep).collect();
        Some(format!("{ELLIPSIS}{kept}"))
    } else {
        Some(tail)
    }
}

pub fn check_detail(outcome: &WatchOutcome, report: Option<&Report>) -> String {
    let exit = describe(outcome);
    match report.and_then(|report| output_tail(&report.text)) {
        Some(tail) => format!("{exit}; last output: {tail}"),
        None => exit,
    }
}

impl StopReason {
    pub fn check_failed(step: usize) -> Self {
        StopReason::CheckFailed { step, detail: None }
    }

    pub fn notice(self, run: &Run, kept: Option<&RunFile>) -> String {
        let why = match self {
            StopReason::Off => return OFF_NOTICE.to_string(),
            StopReason::TurnCap => format!("turn cap reached after {} turns", run.turns),
            StopReason::CostCap => format!("cost cap reached at ${:.2}", run.spent_usd),
            StopReason::Unmet { step } => format!("step {step} missed its exit criterion"),
            StopReason::WorkerFlagged => "a worker asked a question or is blocked".to_string(),
            StopReason::JudgeRounds { step, evidence } => format!(
                "step {step} was still not met after the last judge round: {}",
                evidence.trim_end_matches('.')
            ),
            StopReason::CheckFailed { step, detail } => {
                let detail = detail.map_or(String::new(), |detail| {
                    format!(" ({})", detail.trim_end_matches('.'))
                });
                format!("step {step} check did not exit 0{detail}")
            }
            StopReason::CheckNotStarted { step, error } => format!(
                "step {step} check could not start: {}",
                error.trim_end_matches('.')
            ),
            StopReason::PlanChanged => "the run plan changed or lost its approval".to_string(),
            StopReason::NoReply => "a turn ended without a reply".to_string(),
            StopReason::NeedsYou => "a schedule waits for your approval".to_string(),
            StopReason::TookOver => "you took over".to_string(),
            StopReason::Refused { tool } => format!("the plan does not allow `{tool}`"),
        };
        match kept.and_then(|file| Some((resume_step(file)?, file.plan.steps.len()))) {
            Some((step, steps)) => format!(
                "send-it stopped at step {step}/{steps}: {why}. The plan is kept. Fix the cause, show it with /send-it status, then type /send-it go to resume from step {step}. /send-it off deletes it."
            ),
            None => format!("send-it stopped: {why}. {RESTART_HINT}"),
        }
    }
}

fn flagged(report: &Report) -> bool {
    FLAGGED_STATUSES.contains(&report.status.as_str())
}

pub fn worker_flagged(reports: &[Report]) -> bool {
    reports.iter().any(flagged)
}

pub fn judge_prompt(plan: &RunPlan, progress: Progress) -> Option<String> {
    let step = plan.steps.get(progress.step)?;
    let Criterion::Judge { question } = &step.criterion else {
        return None;
    };
    Some(format!(
        "{}\n/send-it judge: {question}\nAnswer from what this chat already shows and take no other actions. Call `{STEP_RESULT}` once with `met` and one line of `evidence`, then reply in one short line.",
        step_line(plan, progress.step, step)
    ))
}

pub fn done_notice(plan: &RunPlan, run: &Run) -> String {
    let steps = plan.steps.len();
    format!(
        "send-it done: {steps}/{steps} steps in {} turns, ${:.2}. {RESTART_HINT}",
        run.turns, run.spent_usd
    )
}

pub fn running_line(plan: &RunPlan, driving: &Driving) -> String {
    let limits = granted_limits(plan);
    format!(
        "Running: step {}/{}, {} of {} turns, ${:.2} of ${:.2}.",
        driving.progress.step + 1,
        plan.steps.len(),
        driving.run.turns,
        limits.max_turns,
        driving.run.spent_usd,
        limits.max_usd
    )
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
