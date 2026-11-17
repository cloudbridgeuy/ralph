use super::*;

const PENDING_PREFIX: &str = "Pending:";
const PENDING_NONE: [&str; 3] = ["none", "nothing", "n/a"];
const PENDING_CHARS: usize = 200;
const PENDING_MAX: usize = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    pub last_met: Option<String>,
    pub pending: Vec<String>,
}

fn capped(text: &str) -> String {
    if text.chars().count() <= PENDING_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(PENDING_CHARS - ELLIPSIS.len()).collect();
    format!("{kept}{ELLIPSIS}")
}

pub fn pending_lines(reply: &str) -> Vec<String> {
    reply
        .lines()
        .filter_map(|line| line.trim().strip_prefix(PENDING_PREFIX))
        .map(str::trim)
        .filter(|item| {
            !item.is_empty()
                && !PENDING_NONE
                    .iter()
                    .any(|none| item.eq_ignore_ascii_case(none))
        })
        .map(capped)
        .collect()
}

pub fn with_reply(ticked: Ticked, reply: &str) -> Ticked {
    let mut pending = ticked.driving.tally.pending.clone();
    for item in pending_lines(reply) {
        if pending.len() < PENDING_MAX && !pending.contains(&item) {
            pending.push(item);
        }
    }
    Ticked {
        driving: Driving {
            tally: Tally {
                pending,
                ..ticked.driving.tally
            },
            ..ticked.driving
        },
        ..ticked
    }
}

pub(super) fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or_else(String::new, capped)
}

impl StopReason {
    pub(super) fn flagged_by(report: &Report) -> Self {
        StopReason::WorkerFlagged {
            worker: report.worker.clone(),
            line: first_line(&report.text),
        }
    }
}

impl Ticked {
    pub(super) fn recording(self, evidence: Option<&str>) -> Ticked {
        if self.met.is_none() {
            return self;
        }
        let tally = Tally {
            last_met: evidence.map(str::to_string),
            ..self.driving.tally.clone()
        };
        Ticked {
            driving: Driving {
                tally,
                ..self.driving
            },
            ..self
        }
    }
}

const LABEL_WIDTH: usize = 10;
const RULE: &str = "━";
const MODEL_TAG: &str = "[model]";
const CHECK_LABEL: &str = "CHECK FAILED";

type Row = (&'static str, String);

fn count(amount: u32, noun: &str) -> String {
    let plural = if amount == 1 { "" } else { "s" };
    format!("{amount} {noun}{plural}")
}

fn header(title: &str, columns: usize) -> String {
    let head = format!("{RULE}{RULE} {title} ");
    let fill = columns.saturating_sub(head.width()).max(2);
    format!("{head}{}", RULE.repeat(fill))
}

fn labelled(first: &'static str, second: &'static str, texts: Vec<String>) -> Vec<Row> {
    texts
        .into_iter()
        .enumerate()
        .map(|(index, text)| match index {
            0 => (first, text),
            1 => (second, text),
            _ => ("", text),
        })
        .collect()
}

fn render_block(title: &str, rows: &[Row], columns: usize) -> String {
    let body = rows.iter().flat_map(|(label, text)| {
        let mut parts = wrap(text, columns.saturating_sub(LABEL_WIDTH));
        if parts.is_empty() {
            parts.push(String::new());
        }
        parts.into_iter().enumerate().map(move |(index, part)| {
            let label = if index == 0 { *label } else { "" };
            format!("{label:<LABEL_WIDTH$}{part}")
                .trim_end()
                .to_string()
        })
    });
    std::iter::once(header(title, columns))
        .chain(body)
        .collect::<Vec<_>>()
        .join("\n")
}

fn steps_line(count: usize) -> String {
    let range = if count == 1 {
        "1".to_string()
    } else {
        format!("1-{count}")
    };
    let noun = if count == 1 { "step" } else { "steps" };
    format!("✓ {range} ({count} {noun})")
}

fn changed_rows(plan: &RunPlan, met: usize, evidence: Option<&str>) -> Vec<Row> {
    let shown = evidence
        .zip(plan.steps.get(met.wrapping_sub(1)))
        .map(|(evidence, step)| format!("✓ {met} {} — {evidence}", step.task));
    let collapsed = met - usize::from(shown.is_some());
    let texts: Vec<String> = Some(steps_line(collapsed))
        .filter(|_| collapsed > 0)
        .into_iter()
        .chain(shown)
        .collect();
    if texts.is_empty() {
        return vec![("Changed", "nothing".to_string())];
    }
    labelled("Changed", "", texts)
}

fn pending_rows(own: &[String], model: &[String], idle: &str) -> Vec<Row> {
    let texts: Vec<String> = own
        .iter()
        .cloned()
        .chain(model.iter().map(|item| format!("{item} {MODEL_TAG}")))
        .enumerate()
        .map(|(index, item)| format!("{}. {item}", index + 1))
        .collect();
    if texts.is_empty() {
        return vec![("Pending", idle.to_string())];
    }
    labelled("Pending", "on you", texts)
}

pub fn done_block(plan: &RunPlan, driving: &Driving, columns: usize) -> String {
    let steps = plan.steps.len();
    let title = format!(
        "send-it DONE · {steps}/{steps} steps · {} · ${:.2}",
        count(driving.run.turns, "turn"),
        driving.run.spent_usd
    );
    let next = match driving.tally.pending.first() {
        Some(first) => format!("decide item 1: {first}"),
        None => "nothing to do · /send-it off deletes the plan".to_string(),
    };
    let rows: Vec<Row> = changed_rows(plan, steps, driving.tally.last_met.as_deref())
        .into_iter()
        .chain(pending_rows(&[], &driving.tally.pending, "nothing"))
        .chain([("Next", next)])
        .collect();
    render_block(&title, &rows, columns)
}

struct StopView {
    label: &'static str,
    output: Vec<String>,
    why: String,
    tail: String,
    mark: &'static str,
    detail: String,
    own: Vec<String>,
    idle: &'static str,
    next: String,
}

fn unquoted_stop(text: &str) -> String {
    text.trim_end_matches('.').to_string()
}

fn check_command(plan: Option<&RunPlan>, step: usize) -> String {
    plan.and_then(|plan| plan.steps.get(step - 1))
        .and_then(|step| match &step.criterion {
            Criterion::CheckPasses { command } | Criterion::WatchPasses { command } => {
                Some(command.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| "the check".to_string())
}

fn split_detail(detail: Option<&str>) -> (String, Vec<String>) {
    let Some(detail) = detail else {
        return ("did not exit 0".to_string(), Vec::new());
    };
    match detail.split_once(LAST_OUTPUT) {
        Some((exit, output)) => (
            unquoted_stop(exit),
            output.split(OUTPUT_JOIN).map(str::to_string).collect(),
        ),
        None => (unquoted_stop(detail), Vec::new()),
    }
}

fn stop_view(
    reason: &StopReason,
    plan: Option<&RunPlan>,
    step: usize,
    driving: &Driving,
) -> StopView {
    let run = &driving.run;
    let turns = count(run.turns, "turn");
    let spent = format!("${:.2}", run.spent_usd);
    let both = format!("{turns} · {spent}");
    let resume = match plan {
        Some(_) => format!("/send-it go · resumes at step {step}"),
        None => "/send-it status, then /send-it go".to_string(),
    };
    let view = |why: String, mark, detail: String, own: Vec<String>, next: String| StopView {
        label: "STOPPED",
        output: Vec::new(),
        why,
        tail: both.clone(),
        mark,
        detail,
        own,
        idle: "nothing",
        next,
    };
    let unfinished = || "not finished".to_string();
    let rerun = match plan {
        Some(_) => format!("/send-it go · runs the check again at step {step}"),
        None => resume.clone(),
    };
    match reason {
        StopReason::TurnCap => StopView {
            tail: spent.clone(),
            idle: "nothing; the cause is the cap",
            ..view(
                format!("turn cap reached ({turns})"),
                "…",
                unfinished(),
                vec![],
                format!("{resume} with fresh caps"),
            )
        },
        StopReason::CostCap => StopView {
            tail: turns.clone(),
            idle: "nothing; the cause is the cap",
            ..view(
                format!("cost cap reached ({spent})"),
                "…",
                unfinished(),
                vec![],
                format!("{resume} with fresh caps"),
            )
        },
        StopReason::Unmet { .. } => view(
            format!("step {step} missed its exit criterion"),
            "✗",
            "exit criterion not met".to_string(),
            vec![format!("fix step {step} or edit it")],
            resume,
        ),
        StopReason::WorkerFlagged { worker, line } => {
            let said = if line.is_empty() {
                worker.clone()
            } else {
                format!("{worker}: \"{line}\"")
            };
            let then = match plan {
                Some(_) => format!("then /send-it go resumes at step {step}"),
                None => "then /send-it go".to_string(),
            };
            view(
                format!("worker {worker} needs an answer"),
                "✗",
                said,
                vec![format!("answer {worker}")],
                format!("@{worker} <your answer> · {then}"),
            )
        }
        StopReason::JudgeRounds { evidence, .. } => {
            let rounds = plan.map_or_else(
                || "the last judge round".to_string(),
                |plan| format!("{} judge rounds", plan.judge_rounds),
            );
            view(
                format!("not met after {rounds}"),
                "✗",
                unquoted_stop(evidence),
                vec![format!("act on the judge evidence or edit step {step}")],
                resume,
            )
        }
        StopReason::CheckFailed { detail, .. } => {
            let (exit, output) = split_detail(detail.as_deref());
            StopView {
                label: CHECK_LABEL,
                output,
                ..view(
                    format!("`{}` {exit}", check_command(plan, step)),
                    "✗",
                    String::new(),
                    vec!["fix the check failure".to_string()],
                    rerun,
                )
            }
        }
        StopReason::CheckNotStarted { error, .. } => StopView {
            label: CHECK_LABEL,
            ..view(
                format!("`{}` could not start", check_command(plan, step)),
                "✗",
                unquoted_stop(error),
                vec!["fix the check command".to_string()],
                rerun,
            )
        },
        StopReason::PlanChanged => view(
            "the plan changed or lost its approval".to_string(),
            "…",
            unfinished(),
            vec!["review the plan".to_string()],
            "/send-it status, then /send-it go".to_string(),
        ),
        StopReason::NoReply => view(
            "a turn ended without a reply".to_string(),
            "…",
            unfinished(),
            vec!["check the last turn".to_string()],
            resume,
        ),
        StopReason::NeedsYou => view(
            "a schedule waits for your approval".to_string(),
            "…",
            unfinished(),
            vec!["approve the schedule".to_string()],
            resume,
        ),
        StopReason::TookOver => view(
            "you took over".to_string(),
            "…",
            unfinished(),
            vec![],
            resume,
        ),
        StopReason::Refused { tool } => view(
            format!("the plan does not allow `{tool}`"),
            "…",
            unfinished(),
            vec![format!("allow `{tool}` in the plan or edit it")],
            "/send-it edit, then /send-it go".to_string(),
        ),
        StopReason::Off => view(String::new(), "…", unfinished(), vec![], resume),
    }
}

pub fn stop_block(
    reason: &StopReason,
    kept: Option<&RunFile>,
    driving: &Driving,
    columns: usize,
) -> String {
    if *reason == StopReason::Off {
        return OFF_NOTICE.to_string();
    }
    let known = kept.and_then(|file| Some((&file.plan, resume_step(file)?)));
    let step = known.map_or(driving.progress.step + 1, |(_, step)| step);
    let view = stop_view(reason, known.map(|(plan, _)| plan), step, driving);
    let place = known.map_or(String::new(), |(plan, step)| {
        format!(" at {step}/{}", plan.steps.len())
    });
    let title = format!(
        "send-it {}{place} · {} · {}",
        view.label, view.why, view.tail
    );
    let changed = known.map_or_else(Vec::new, |(plan, step)| {
        let task = plan
            .steps
            .get(step - 1)
            .map_or("", |step| step.task.as_str());
        let failed = match view.detail.as_str() {
            "" => format!("{} {step} {task}", view.mark),
            detail => format!("{} {step} {task} — {detail}", view.mark),
        };
        let texts = Some(steps_line(step - 1))
            .filter(|_| step > 1)
            .into_iter()
            .chain([failed])
            .collect();
        labelled("Changed", "", texts)
    });
    let rows: Vec<Row> = changed
        .into_iter()
        .chain(labelled("Output", "", view.output))
        .chain(pending_rows(&view.own, &driving.tally.pending, view.idle))
        .chain([("Next", view.next)])
        .collect();
    render_block(&title, &rows, columns)
}

impl Ticked {
    pub fn shown_met(&self) -> Option<&str> {
        self.met.as_deref().filter(|_| self.next != Next::Done)
    }
}

pub fn run_end_prefill(next: &Next) -> Option<String> {
    match next {
        Next::Send(_)
        | Next::Hold(_)
        | Next::Watch(_)
        | Next::Check(_)
        | Next::Judge
        | Next::Wait
        | Next::Done => None,
        Next::Stop(reason) => match reason {
            StopReason::Off => None,
            StopReason::PlanChanged => Some("/send-it status".to_string()),
            StopReason::TurnCap
            | StopReason::CostCap
            | StopReason::Unmet { .. }
            | StopReason::WorkerFlagged { .. }
            | StopReason::JudgeRounds { .. }
            | StopReason::CheckFailed { .. }
            | StopReason::CheckNotStarted { .. }
            | StopReason::NoReply
            | StopReason::NeedsYou
            | StopReason::TookOver
            | StopReason::Refused { .. } => Some("/send-it go".to_string()),
        },
    }
}
