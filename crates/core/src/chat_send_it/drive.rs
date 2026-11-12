use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub step: usize,
    pub judge_rounds: u32,
    pub probing: bool,
}

pub struct Seen<'a> {
    pub last: &'a HashMap<String, String>,
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
        Criterion::WorkerDone { worker } => Next::Hold(worker.clone()),
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

fn step_met(plan: &RunPlan, progress: Progress) -> (Progress, Next) {
    let index = progress.step + 1;
    match plan.steps.get(index) {
        Some(step) => (
            Progress {
                step: index,
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
                wait_on(criterion)
            };
            (progress, next)
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
                        step: progress.step,
                        judge_rounds,
                        probing: false,
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
}

pub struct Ended<'a> {
    pub file: Option<&'a RunFile>,
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

fn probe(plan: &RunPlan, progress: Progress, seen: &Seen) -> (Progress, Next) {
    let Some(step) = plan.steps.get(progress.step) else {
        return (progress, Next::Done);
    };
    match &step.criterion {
        Criterion::WorkerDone { worker } => match seen.last.get(worker) {
            Some(status) if status == "done" => step_met(plan, progress),
            known if known.is_some() || seen.running.contains(worker) => {
                (progress, Next::Hold(worker.clone()))
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

pub fn start_run(file: &RunFile, seen: &Seen) -> Option<Ticked> {
    if !file.is_approved() {
        return None;
    }
    let hash = plan_hash(&file.plan);
    let start = Progress {
        step: resume_step(file).map_or(0, |step| step - 1),
        ..Progress::default()
    };
    let first = match resume_step(file) {
        Some(_) => probe(&file.plan, start, seen),
        None => advance(&file.plan, start, Event::Start),
    };
    let (progress, next) = match first {
        (progress, Next::Send(line)) => (
            progress,
            Next::Send(format!(
                "send-it run {hash} is approved. Work the current step only, and take only the allowed actions; ralph checks each exit criterion and sends the next step.\n\n{}\n\n{line}",
                render_run(&file.plan)
            )),
        ),
        started => started,
    };
    let met = (progress.step > start.step || next == Next::Done)
        .then(|| met_line(&file.plan, start.step, None));
    let driving = Driving {
        hash,
        awaiting: awaiting(&next, progress),
        progress,
        run: Run::default(),
    };
    Some(Ticked { driving, next, met })
}

fn is_off(line: &str) -> bool {
    Command::SendIt.args(line.trim()).map(parse_send_it_args) == Some(SendItCommand::Off)
}

fn unanswered_flag(ended: &Ended, file: &RunFile) -> bool {
    let may_reply = file.plan.allowed.contains(&Action::Reply);
    worker_flagged(ended.unread)
        || ended
            .drained
            .iter()
            .filter(|report| flagged(report))
            .any(|report| !(may_reply && ended.answered.contains(&report.worker)))
}

fn current_plan<'a>(file: Option<&'a RunFile>, hash: &str) -> Result<&'a RunFile, StopReason> {
    let file = file.ok_or(StopReason::Off)?;
    if file.is_approved() && plan_hash(&file.plan) == hash {
        Ok(file)
    } else {
        Err(StopReason::PlanChanged)
    }
}

fn admitted<'a>(ended: &Ended<'a>, hash: &str) -> Result<&'a RunFile, StopReason> {
    let file = current_plan(
        ended
            .file
            .filter(|_| !ended.queued.iter().any(|line| is_off(line))),
        hash,
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
    } else if unanswered_flag(ended, file) {
        Some(StopReason::WorkerFlagged)
    } else {
        None
    };
    stop.map_or(Ok(file), Err)
}

fn events<'a>(awaiting: &Awaiting, ended: &Ended<'a>) -> Vec<Event<'a>> {
    match awaiting {
        Awaiting::Turn => Vec::new(),
        Awaiting::Verdict => vec![Event::Verdict(ended.step_result)],
        Awaiting::Report(source) => ended
            .watched
            .map(Event::Watched)
            .into_iter()
            .chain(
                ended
                    .drained
                    .iter()
                    .filter(|report| report.worker == *source && !flagged(report))
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
        },
        next,
        met,
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
    if matches!(decided, Next::Done | Next::Stop(_)) {
        return finish(run, progress, decided, met);
    }
    let next = match advance(plan, progress, Event::TurnEnded(&run)).1 {
        stop @ Next::Stop(_) => stop,
        waiting if decided == Next::Wait => waiting,
        _ => decided,
    };
    finish(run, progress, next, met)
}

pub fn awaits_check(driving: &Driving) -> bool {
    matches!(&driving.awaiting, Awaiting::Report(source) if is_check_watch_id(source))
}

pub fn settle_check(
    driving: &Driving,
    file: Option<&RunFile>,
    outcome: &WatchOutcome,
    reports: &[Report],
) -> Ticked {
    let finish = |progress: Progress, next: Next, met: Option<String>| Ticked {
        driving: Driving {
            hash: driving.hash.clone(),
            progress,
            run: driving.run.clone(),
            awaiting: awaiting(&next, progress),
        },
        next,
        met,
    };
    let plan = match current_plan(file, &driving.hash) {
        Ok(file) => &file.plan,
        Err(reason) => return finish(driving.progress, Next::Stop(reason), None),
    };
    let (progress, decided) = advance(plan, driving.progress, Event::Watched(outcome));
    let source = check_watch_id(driving.progress.step);
    let decided = match decided {
        Next::Stop(StopReason::CheckFailed { step, .. }) => {
            let report = reports.iter().find(|report| report.worker == source);
            Next::Stop(StopReason::CheckFailed {
                step,
                detail: Some(check_detail(outcome, report)),
            })
        }
        other => other,
    };
    let met = (progress.step > driving.progress.step || decided == Next::Done)
        .then(|| met_line(plan, driving.progress.step, None));
    let next = match decided {
        Next::Done | Next::Stop(_) => decided,
        _ => match advance(plan, progress, Event::TurnEnded(&driving.run)).1 {
            stop @ Next::Stop(_) => stop,
            _ => decided,
        },
    };
    finish(progress, next, met)
}

fn met_line(plan: &RunPlan, step: usize, evidence: Option<&str>) -> String {
    let label = plan
        .steps
        .get(step)
        .map(|step| step.criterion.label())
        .unwrap_or_default();
    let evidence = evidence.map_or(String::new(), |evidence| format!(": {evidence}"));
    format!(
        "send-it: step {}/{} met ({label}){evidence}",
        step + 1,
        plan.steps.len()
    )
}
