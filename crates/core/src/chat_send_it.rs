use crate::inbox_alert::{Alert, AlertConfig};
use crate::program_status::{Kind, Report as ProgramReport, State};
use crate::watch::{watch_id, WatchKind, WatchOutcome, WatchSpec};
use crate::worker::{HarnessId, Report};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const DEFAULT_JUDGE_ROUNDS: u32 = 2;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendItCommand {
    Usage,
    Propose(String),
    Edit,
    Go,
    Status,
    Off,
}

pub fn parse_send_it_args(args: &str) -> SendItCommand {
    match args.trim() {
        "" => SendItCommand::Usage,
        "edit" => SendItCommand::Edit,
        "go" => SendItCommand::Go,
        "status" => SendItCommand::Status,
        "off" => SendItCommand::Off,
        text => SendItCommand::Propose(text.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunFile {
    pub plan: RunPlan,
    pub approved: Option<String>,
}

const APPROVED_KEY: &str = "approved";
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
    })
}

pub fn write_run(file: &RunFile) -> Result<String, String> {
    let unwritable = |error: String| format!("Could not write run.toml: {error}");
    let mut value =
        serde_json::to_value(RawPlan::from(&file.plan)).map_err(|e| unwritable(e.to_string()))?;
    if let (Some(fields), Some(hash)) = (value.as_object_mut(), &file.approved) {
        fields.insert(APPROVED_KEY.to_string(), Value::String(hash.clone()));
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
        plan: file.plan.clone(),
        approved: Some(hash),
    })
}

pub fn render_status(file: &RunFile) -> String {
    let hash = plan_hash(&file.plan);
    let state = if file.is_approved() {
        format!("Plan {hash}: approved.")
    } else {
        format!("Plan {hash}: not approved. Type /send-it go to approve this plan.")
    };
    format!("{}\n{state}", render_run(&file.plan))
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
        "/send-it: restate these instructions as a run plan. Call `propose_run` once with the plan, then stop. Do not start the work: ralph shows the plan and the person approves it with /send-it go.\n\n{text}"
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
                                "description": "Exit check. worker_done needs worker; watch_passes and check_passes need a command that exits 0; judge needs a question.",
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
    CheckFailed { step: usize },
    Off,
    PlanChanged,
    NoReply,
    NeedsYou,
    TookOver,
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
            StopReason::CheckFailed { step } => format!("step {step} check did not exit 0"),
            StopReason::Off => "the run plan is gone".to_string(),
            StopReason::PlanChanged => "the run plan changed or lost its approval".to_string(),
            StopReason::NoReply => "a turn ended without a reply".to_string(),
            StopReason::NeedsYou => "a schedule waits for your approval".to_string(),
            StopReason::TookOver => "you took over".to_string(),
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
    Watched(&'a WatchOutcome),
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
        (
            Event::Watched(outcome),
            Criterion::WatchPasses { .. } | Criterion::CheckPasses { .. },
        ) => {
            if *outcome == WatchOutcome::Exited(0) {
                step_met(plan, progress)
            } else {
                (
                    progress,
                    Next::Stop(StopReason::CheckFailed { step: number }),
                )
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Awaiting {
    Turn,
    Verdict,
    Report(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Driving {
    pub hash: String,
    pub progress: Progress,
    pub run: Run,
    pub awaiting: Awaiting,
}

pub struct Ended<'a> {
    pub file: Option<&'a RunFile>,
    pub replied: bool,
    pub asked: bool,
    pub typed: bool,
    pub cost_usd: f64,
    pub reply: &'a str,
    pub drained: &'a [Report],
    pub unread: &'a [Report],
    pub watched: Option<&'a WatchOutcome>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ticked {
    pub driving: Driving,
    pub next: Next,
    pub met: Option<String>,
}

const CHECK_ID: &str = "send-it-";
const VERDICT_MET: &str = "verdict: met";

pub fn check_watch_id(step: usize) -> String {
    watch_id(&format!("{CHECK_ID}{}", step + 1))
}

pub fn check_spec(step: usize, command: &str) -> WatchSpec {
    WatchSpec {
        id: check_watch_id(step),
        command: command.to_string(),
        follow_up: None,
        kind: WatchKind::Normal,
        timeout: None,
        limit: None,
    }
}

fn awaiting(next: &Next, progress: Progress) -> Awaiting {
    match next {
        Next::Judge => Awaiting::Verdict,
        Next::Hold(worker) => Awaiting::Report(worker.clone()),
        Next::Watch(_) | Next::Check(_) => Awaiting::Report(check_watch_id(progress.step)),
        _ => Awaiting::Turn,
    }
}

pub fn start_run(file: &RunFile) -> Option<(Driving, Next)> {
    if !file.is_approved() {
        return None;
    }
    let (progress, next) = advance(&file.plan, Progress::default(), Event::Start);
    let driving = Driving {
        hash: plan_hash(&file.plan),
        progress,
        run: Run::default(),
        awaiting: Awaiting::Turn,
    };
    Some((driving, next))
}

fn admitted<'a>(ended: &Ended<'a>, hash: &str) -> Result<&'a RunFile, StopReason> {
    let file = ended.file.ok_or(StopReason::Off)?;
    let stop = if !file.is_approved() || plan_hash(&file.plan) != hash {
        Some(StopReason::PlanChanged)
    } else if !ended.replied {
        Some(StopReason::NoReply)
    } else if ended.typed {
        Some(StopReason::TookOver)
    } else if ended.asked {
        Some(StopReason::NeedsYou)
    } else if worker_flagged(ended.unread, ended.drained) {
        Some(StopReason::WorkerFlagged)
    } else {
        None
    };
    stop.map_or(Ok(file), Err)
}

fn events<'a>(awaiting: &Awaiting, ended: &Ended<'a>) -> Vec<Event<'a>> {
    match awaiting {
        Awaiting::Turn => Vec::new(),
        Awaiting::Verdict => vec![Event::Verdict(parse_verdict(ended.reply))],
        Awaiting::Report(source) => ended
            .watched
            .map(Event::Watched)
            .into_iter()
            .chain(
                ended
                    .drained
                    .iter()
                    .filter(|report| report.worker == *source)
                    .map(Event::Report),
            )
            .collect(),
    }
}

fn fold(plan: &RunPlan, progress: Progress, events: &[Event]) -> (Progress, Next) {
    events
        .iter()
        .try_fold(progress, |progress, event| {
            match advance(plan, progress, *event) {
                (progress, Next::Wait) => Ok(progress),
                decided => Err(decided),
            }
        })
        .map_or_else(|decided| decided, |progress| (progress, Next::Wait))
}

pub fn tick(driving: &Driving, ended: &Ended) -> Ticked {
    let run = Run {
        turns: driving.run.turns + 1,
        spent_usd: driving.run.spent_usd + ended.cost_usd,
    };
    let finish = |progress: Progress, next: Next, met: Option<String>| Ticked {
        driving: Driving {
            hash: driving.hash.clone(),
            progress,
            run: run.clone(),
            awaiting: awaiting(&next, progress),
        },
        next,
        met,
    };
    let file = match admitted(ended, &driving.hash) {
        Ok(file) => file,
        Err(reason) => return finish(driving.progress, Next::Stop(reason), None),
    };
    let plan = &file.plan;
    let (progress, decided) = fold(plan, driving.progress, &events(&driving.awaiting, ended));
    let met = (progress.step > driving.progress.step || decided == Next::Done)
        .then(|| met_line(plan, driving.progress.step));
    if matches!(decided, Next::Done | Next::Stop(_)) {
        return finish(progress, decided, met);
    }
    let next = match advance(plan, progress, Event::TurnEnded(&run)).1 {
        stop @ Next::Stop(_) => stop,
        waiting if decided == Next::Wait => waiting,
        _ => decided,
    };
    finish(progress, next, met)
}

fn met_line(plan: &RunPlan, step: usize) -> String {
    let label = plan
        .steps
        .get(step)
        .map(|step| step.criterion.label())
        .unwrap_or_default();
    format!(
        "send-it: step {}/{} met ({label})",
        step + 1,
        plan.steps.len()
    )
}

pub fn judge_prompt(plan: &RunPlan, progress: Progress) -> Option<String> {
    let step = plan.steps.get(progress.step)?;
    let Criterion::Judge { question } = &step.criterion else {
        return None;
    };
    Some(format!(
        "{}\n/send-it judge: {question}\nAnswer from what this chat already shows and take no actions. End your reply with one line: `{VERDICT_MET}` or `verdict: unmet`.",
        step_line(plan, progress.step, step)
    ))
}

pub fn parse_verdict(reply: &str) -> bool {
    reply
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| {
            line.trim_matches(|c: char| c.is_whitespace() || c == '*' || c == '`')
                .eq_ignore_ascii_case(VERDICT_MET)
        })
}

pub fn done_notice(plan: &RunPlan, run: &Run) -> String {
    let steps = plan.steps.len();
    format!(
        "send-it done: {steps}/{steps} steps in {} turns, ${:.2}.",
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
