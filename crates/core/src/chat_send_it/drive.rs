use super::*;
use crate::worker::{branch_name, worktree_for_branch, ReportKind, ReportStatus, WorkerId};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub step: usize,
    pub judge_rounds: u32,
    pub probing: bool,
    pub verify: VerifyMode,
    pub verifying: bool,
}

// verify is Off here: Progress defaults are plain unit state; start_run seeds the real mode from Limits.
impl Default for Progress {
    fn default() -> Self {
        Progress {
            step: 0,
            judge_rounds: 0,
            probing: false,
            verify: VerifyMode::Off,
            verifying: false,
        }
    }
}

pub struct Seen<'a> {
    pub last: &'a HashMap<WorkerId, ReportKind>,
    pub running: &'a [String],
}

#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
    Start,
    TurnEnded(&'a Run),
    Report(&'a Report),
    Watched(&'a WatchOutcome),
    Verdict(Option<&'a StepResult>),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopCause {
    NeedsUser { worker: Sender },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepMove {
    Complete,
    Hold,
    Fail,
    Stop(StopCause),
}

pub fn step_move(report: &Report, awaited: Option<&WorkerId>) -> StepMove {
    let from_awaited = awaited.is_some_and(|id| report.from.worker() == Some(id));
    let unmet = if from_awaited {
        StepMove::Fail
    } else {
        StepMove::Hold
    };
    match report.status {
        ReportKind::Worker(ReportStatus::Blocked | ReportStatus::Question) => {
            StepMove::Stop(StopCause::NeedsUser {
                worker: report.from.clone(),
            })
        }
        ReportKind::Worker(ReportStatus::Done) if from_awaited => StepMove::Complete,
        ReportKind::Worker(ReportStatus::Done) => StepMove::Hold,
        ReportKind::Check
        | ReportKind::Warning
        | ReportKind::Watch
        | ReportKind::Merged
        | ReportKind::View
        | ReportKind::ThreadClosed
        | ReportKind::External => unmet,
    }
}

pub(super) fn granted_limits(plan: &RunPlan) -> Limits {
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

pub fn step_line(plan: &RunPlan, index: usize, step: &Step) -> String {
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
        Criterion::WorkerDone { worker, .. } => Next::Hold(worker.to_string()),
        Criterion::WatchPasses { command } => Next::Watch(command.clone()),
        Criterion::CheckPasses { command } => Next::Check(command.clone()),
        Criterion::Judge { .. } => Next::Judge,
    }
}

fn begin(plan: &RunPlan, index: usize, step: &Step) -> Next {
    match &step.criterion {
        Criterion::WatchPasses { .. } | Criterion::CheckPasses { .. } => wait_on(&step.criterion),
        Criterion::WorkerDone { .. } | Criterion::Judge { .. } => {
            Next::Send(step_line(plan, index, step))
        }
    }
}

fn done_reported(plan: &RunPlan, progress: Progress, criterion: &Criterion) -> (Progress, Next) {
    match criterion {
        Criterion::WorkerDone {
            verify: Some(command),
            ..
        } if progress.verify != VerifyMode::Off => (
            Progress {
                verifying: true,
                ..progress
            },
            Next::Check(command.clone()),
        ),
        _ => step_met(plan, progress),
    }
}

fn step_met(plan: &RunPlan, progress: Progress) -> (Progress, Next) {
    let index = progress.step + 1;
    match plan.steps.get(index) {
        Some(step) => (
            Progress {
                step: index,
                verify: progress.verify,
                ..Progress::default()
            },
            begin(plan, index, step),
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
        (Event::Start, _) => (progress, begin(plan, progress.step, step)),
        (Event::TurnEnded(run), criterion) => {
            let limits = granted_limits(plan);
            let next = if run.turns >= limits.max_turns {
                Next::Stop(StopReason::TurnCap)
            } else if run.spent_usd >= limits.max_usd {
                Next::Stop(StopReason::CostCap)
            } else {
                match criterion {
                    Criterion::WorkerDone {
                        verify: Some(command),
                        ..
                    } if progress.verifying => Next::Check(command.clone()),
                    _ => wait_on(criterion),
                }
            };
            (progress, next)
        }
        (Event::Report(report), Criterion::WorkerDone { worker, .. }) => {
            match step_move(report, Some(worker)) {
                StepMove::Complete => done_reported(plan, progress, &step.criterion),
                StepMove::Fail => unmet,
                StepMove::Hold => (progress, Next::Wait),
                StepMove::Stop(cause) => {
                    (progress, Next::Stop(StopReason::stopped_by(cause, report)))
                }
            }
        }
        (
            Event::Watched(outcome),
            Criterion::WatchPasses { .. } | Criterion::CheckPasses { .. },
        ) => {
            if *outcome == WatchOutcome::Exited(0) {
                step_met(plan, progress)
            } else {
                (progress, Next::Stop(StopReason::check_failed(number)))
            }
        }
        (Event::Watched(outcome), Criterion::WorkerDone { .. }) if progress.verifying => {
            if *outcome == WatchOutcome::Exited(0) || progress.verify == VerifyMode::Observe {
                step_met(plan, progress)
            } else {
                (progress, Next::Stop(StopReason::check_failed(number)))
            }
        }
        (Event::Verdict(Some(result)), Criterion::Judge { .. }) if result.met => {
            step_met(plan, progress)
        }
        (Event::Verdict(result), Criterion::Judge { .. }) => {
            let evidence = result.map_or(NO_STEP_RESULT, |result| &result.evidence);
            let judge_rounds = progress.judge_rounds + u32::from(!progress.probing);
            if judge_rounds >= plan.judge_rounds {
                (
                    progress,
                    Next::Stop(StopReason::JudgeRounds {
                        step: number,
                        evidence: evidence.to_string(),
                    }),
                )
            } else {
                (
                    Progress {
                        judge_rounds,
                        probing: false,
                        ..progress
                    },
                    Next::Send(format!(
                        "{}\njudge: not met: {evidence}",
                        step_line(plan, progress.step, step)
                    )),
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
    pub briefed: bool,
    pub tally: Tally,
}

pub struct Ended<'a> {
    pub file: Option<&'a RunFile>,
    pub trust: Trust<'a>,
    pub replied: bool,
    pub asked: bool,
    pub typed: bool,
    pub queued: &'a [String],
    pub cost_usd: f64,
    pub step_result: Option<&'a StepResult>,
    pub drained: &'a [Report],
    pub unread: &'a [Report],
    pub answered: &'a [String],
    pub watched: Option<&'a WatchOutcome>,
    pub refused: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ticked {
    pub driving: Driving,
    pub next: Next,
    pub met: Option<String>,
    pub notice: Option<String>,
}

const CHECK_ID: &str = "send-it-";

pub fn check_watch_id(step: usize) -> String {
    watch_id(&format!("{CHECK_ID}{}", step + 1))
}

pub fn is_check_watch_id(id: &str) -> bool {
    id.strip_prefix(&watch_id(CHECK_ID))
        .is_some_and(|step| !step.is_empty() && step.bytes().all(|b| b.is_ascii_digit()))
}

pub fn check_spec(step: usize, command: &str) -> WatchSpec {
    WatchSpec {
        id: check_watch_id(step),
        command: command.to_string(),
        follow_up: None,
        kind: WatchKind::Normal,
        timeout: None,
        limit: None,
        cwd: None,
    }
}

pub fn verify_spec(step: usize, command: &str, cwd: PathBuf) -> WatchSpec {
    WatchSpec {
        cwd: Some(cwd),
        ..check_spec(step, command)
    }
}

pub fn verify_worker(plan: &RunPlan, progress: Progress) -> Option<&str> {
    match plan.steps.get(progress.step).map(|step| &step.criterion) {
        Some(Criterion::WorkerDone { worker, .. }) if progress.verifying => Some(worker.as_str()),
        _ => None,
    }
}

pub fn resolve_check(
    step: usize,
    command: &str,
    worker: Option<&str>,
    porcelain: Option<&str>,
) -> Result<WatchSpec, StopReason> {
    let Some(worker) = worker else {
        return Ok(check_spec(step, command));
    };
    porcelain
        .and_then(|listing| verify_cwd(listing, worker))
        .map(|cwd| verify_spec(step, command, cwd))
        .ok_or_else(|| StopReason::CheckNotStarted {
            step: step + 1,
            error: format!(
                "No worktree found for worker {worker}, so the verify command has no place to run."
            ),
        })
}

fn awaiting(next: &Next, progress: Progress) -> Awaiting {
    match next {
        Next::Judge => Awaiting::Verdict,
        Next::Hold(worker) => Awaiting::Report(worker.clone()),
        Next::Watch(_) | Next::Check(_) => Awaiting::Report(check_watch_id(progress.step)),
        _ => Awaiting::Turn,
    }
}

fn probe(plan: &RunPlan, progress: Progress, seen: &Seen) -> (Progress, Next) {
    let Some(step) = plan.steps.get(progress.step) else {
        return (progress, Next::Done);
    };
    match &step.criterion {
        Criterion::WorkerDone { worker, .. } => match seen.last.get(worker) {
            Some(ReportKind::Worker(ReportStatus::Done)) => {
                done_reported(plan, progress, &step.criterion)
            }
            known if known.is_some() || seen.running.iter().any(|id| id == worker.as_str()) => {
                (progress, Next::Hold(worker.to_string()))
            }
            _ => advance(plan, progress, Event::Start),
        },
        Criterion::Judge { .. } => (
            Progress {
                probing: true,
                ..progress
            },
            Next::Judge,
        ),
        criterion => (progress, wait_on(criterion)),
    }
}

pub fn start_run(file: &RunFile, seen: &Seen, trust: Trust) -> Option<Ticked> {
    if !file.is_approved(trust) {
        return None;
    }
    let hash = plan_hash(&file.plan);
    let start = Progress {
        step: resume_step(file).map_or(0, |step| step - 1),
        verify: file.plan.limits.verify,
        ..Progress::default()
    };
    let first = match resume_step(file) {
        Some(_) => probe(&file.plan, start, seen),
        None => advance(&file.plan, start, Event::Start),
    };
    let (progress, next) = first;
    let met = (progress.step > start.step || next == Next::Done)
        .then(|| met_line(&file.plan, start.step, None));
    let driving = Driving {
        hash,
        awaiting: awaiting(&next, progress),
        progress,
        run: Run::default(),
        briefed: false,
        tally: Tally::default(),
    };
    Some(
        Ticked {
            driving,
            next,
            met,
            notice: None,
        }
        .briefed_by(&file.plan),
    )
}

impl Ticked {
    fn noticing(self, notice: Option<String>) -> Ticked {
        Ticked { notice, ..self }
    }

    fn briefed_by(self, plan: &RunPlan) -> Ticked {
        match self.next {
            Next::Send(line) => {
                let (driving, line) = briefed_turn(plan, self.driving, &line);
                Ticked {
                    driving,
                    next: Next::Send(line),
                    met: self.met,
                    notice: self.notice,
                }
            }
            next => Ticked { next, ..self },
        }
    }
}

pub fn briefed_turn(plan: &RunPlan, driving: Driving, line: &str) -> (Driving, String) {
    if driving.briefed {
        return (driving, line.to_string());
    }
    let text = brief(plan, &driving.hash, line);
    (
        Driving {
            briefed: true,
            ..driving
        },
        text,
    )
}

fn brief(plan: &RunPlan, hash: &str, line: &str) -> String {
    format!(
        "send-it run {hash} is approved. Work the current step only, and take only the allowed actions; ralph checks each exit criterion and sends the next step. End each reply with one `Pending: <one line>` line per follow-up the person must still decide or do, such as unpushed commits or unrun checks, and no `Next step:` line; ralph prints the next step.\n\n{}\n\n{line}",
        render_run(plan)
    )
}

fn is_off(line: &str) -> bool {
    Command::SendIt.args(line.trim()).map(parse_send_it_args) == Some(SendItCommand::Off)
}

fn unanswered_flag<'a>(ended: &Ended<'a>, file: &RunFile) -> Option<&'a Report> {
    let may_reply = file.plan.allowed.contains(&Action::Reply);
    ended
        .unread
        .iter()
        .find(|report| flagged(report))
        .or_else(|| {
            ended
                .drained
                .iter()
                .filter(|report| flagged(report))
                .find(|report| !(may_reply && ended.answered.contains(&report.from.to_string())))
        })
}

fn current_plan<'a>(
    file: Option<&'a RunFile>,
    hash: &str,
    trust: Trust,
) -> Result<&'a RunFile, StopReason> {
    let file = file.ok_or(StopReason::Off)?;
    if file.is_approved(trust) && plan_hash(&file.plan) == hash {
        Ok(file)
    } else {
        Err(StopReason::PlanChanged(approval_refusal(file, trust)))
    }
}

fn admitted<'a>(ended: &Ended<'a>, hash: &str) -> Result<&'a RunFile, StopReason> {
    let file = current_plan(
        ended
            .file
            .filter(|_| !ended.queued.iter().any(|line| is_off(line))),
        hash,
        ended.trust,
    )?;
    let stop = if let Some(tool) = ended.refused {
        Some(StopReason::Refused {
            tool: tool.to_string(),
        })
    } else if !ended.replied {
        Some(StopReason::NoReply)
    } else if ended.typed || !ended.queued.is_empty() {
        Some(StopReason::TookOver)
    } else if ended.asked {
        Some(StopReason::NeedsYou)
    } else {
        unanswered_flag(ended, file).map(StopReason::flagged_by)
    };
    stop.map_or(Ok(file), Err)
}

fn events<'a>(awaiting: &Awaiting, ended: &Ended<'a>) -> Vec<Event<'a>> {
    match awaiting {
        Awaiting::Turn => Vec::new(),
        Awaiting::Verdict => vec![Event::Verdict(ended.step_result)],
        // Reached when a resume finds a stale report from a stopped run's check; do not delete as dead code.
        Awaiting::Report(source) => ended
            .watched
            .map(Event::Watched)
            .into_iter()
            .chain(
                ended
                    .drained
                    .iter()
                    .filter(|report| report.from.to_string() == *source && !flagged(report))
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
    let counted = Run {
        turns: driving.run.turns + 1,
        spent_usd: driving.run.spent_usd + ended.cost_usd,
    };
    let finish = |run: Run, progress: Progress, next: Next, met: Option<String>| Ticked {
        driving: Driving {
            hash: driving.hash.clone(),
            progress,
            run,
            awaiting: awaiting(&next, progress),
            briefed: driving.briefed,
            tally: driving.tally.clone(),
        },
        next,
        met,
        notice: None,
    };
    let file = match admitted(ended, &driving.hash) {
        Ok(file) => file,
        Err(reason) => return finish(counted, driving.progress, Next::Stop(reason), None),
    };
    let plan = &file.plan;
    let (progress, decided) = fold(plan, driving.progress, &events(&driving.awaiting, ended));
    let settled_report = matches!(driving.awaiting, Awaiting::Report(_)) && decided != Next::Wait;
    let run = if settled_report {
        Run {
            turns: driving.run.turns,
            ..counted
        }
    } else {
        counted
    };
    let evidence = (driving.awaiting == Awaiting::Verdict)
        .then_some(ended.step_result)
        .flatten()
        .map(|result| result.evidence.as_str());
    let met = (progress.step > driving.progress.step || decided == Next::Done)
        .then(|| met_line(plan, driving.progress.step, evidence));
    let notice = ended
        .watched
        .filter(|_| awaits_check(driving))
        .and_then(|outcome| would_block(driving.progress, outcome, ended.drained));
    if matches!(decided, Next::Done | Next::Stop(_)) {
        return finish(run, progress, decided, met)
            .noticing(notice)
            .recording(evidence);
    }
    let next = match advance(plan, progress, Event::TurnEnded(&run)).1 {
        stop @ Next::Stop(_) => stop,
        waiting if decided == Next::Wait => waiting,
        _ => decided,
    };
    finish(run, progress, next, met)
        .noticing(notice)
        .briefed_by(plan)
        .recording(evidence)
}

pub fn awaits_check(driving: &Driving) -> bool {
    matches!(&driving.awaiting, Awaiting::Report(source) if is_check_watch_id(source))
}

pub fn settle_check(
    driving: &Driving,
    file: Option<&RunFile>,
    outcome: &WatchOutcome,
    reports: &[Report],
    trust: Trust,
) -> Ticked {
    let finish = |progress: Progress, next: Next, met: Option<String>| Ticked {
        driving: Driving {
            hash: driving.hash.clone(),
            progress,
            run: driving.run.clone(),
            awaiting: awaiting(&next, progress),
            briefed: driving.briefed,
            tally: driving.tally.clone(),
        },
        next,
        met,
        notice: None,
    };
    let plan = match current_plan(file, &driving.hash, trust) {
        Ok(file) => &file.plan,
        Err(reason) => return finish(driving.progress, Next::Stop(reason), None),
    };
    let (progress, decided) = advance(plan, driving.progress, Event::Watched(outcome));
    let source = check_watch_id(driving.progress.step);
    let decided = match decided {
        Next::Stop(StopReason::CheckFailed { step, .. }) => {
            let report = reports
                .iter()
                .find(|report| report.from.to_string() == source);
            Next::Stop(StopReason::CheckFailed {
                step,
                detail: Some(check_detail(outcome, report)),
            })
        }
        other => other,
    };
    let met = (progress.step > driving.progress.step || decided == Next::Done)
        .then(|| met_line(plan, driving.progress.step, None));
    let notice = would_block(driving.progress, outcome, reports);
    let next = match decided {
        Next::Done | Next::Stop(_) => decided,
        _ => match advance(plan, progress, Event::TurnEnded(&driving.run)).1 {
            stop @ Next::Stop(_) => stop,
            _ => decided,
        },
    };
    finish(progress, next, met)
        .noticing(notice)
        .briefed_by(plan)
        .recording(None)
}

fn would_block(progress: Progress, outcome: &WatchOutcome, reports: &[Report]) -> Option<String> {
    (progress.verifying
        && progress.verify == VerifyMode::Observe
        && *outcome != WatchOutcome::Exited(0))
    .then(|| {
        let source = check_watch_id(progress.step);
        let report = reports
            .iter()
            .find(|report| report.from.to_string() == source);
        would_block_line(progress.step, &check_detail(outcome, report))
    })
}

pub fn would_block_line(step: usize, detail: &str) -> String {
    format!(
        "send-it: step {} would block: verify failed: {detail}",
        step + 1
    )
}

pub fn verify_cwd(porcelain: &str, worker: &str) -> Option<PathBuf> {
    worktree_for_branch(porcelain, &branch_name(worker))
}

fn met_line(plan: &RunPlan, step: usize, evidence: Option<&str>) -> String {
    let label = match plan.steps.get(step).map(|step| &step.criterion) {
        Some(Criterion::Judge { .. }) | None => String::new(),
        Some(criterion) => format!(" ({})", criterion.label()),
    };
    let evidence = evidence.map_or(String::new(), |evidence| format!(": {evidence}"));
    format!(
        "send-it: step {}/{} met{label}{evidence}",
        step + 1,
        plan.steps.len()
    )
}
