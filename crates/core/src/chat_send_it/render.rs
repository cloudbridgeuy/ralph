use super::{
    hang_wrapped, plan_hash, plan_warnings, resume_step, Action, Criterion, RunFile, RunPlan, Step,
    RUN_BY_RALPH_NOTE,
};
use crate::ansi::{BOLD, CYAN as CODE, DIM, GREEN, RESET, YELLOW};
use crate::chat_help::wrap;

const BRIEF_WIDTH: usize = 68;
const BRIEF_INDENT: &str = "       ";
const NO_LIMIT: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Plain,
    Header,
    Brief,
    Exit,
    Dim,
    Warning,
    Footer(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    role: Role,
    text: String,
}

impl Row {
    fn new(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
        }
    }
}

fn step_kind(step: &Step) -> &'static str {
    match &step.criterion {
        Criterion::WorkerDone { .. } => {
            match step
                .task
                .split_whitespace()
                .next()
                .map(str::to_lowercase)
                .as_deref()
            {
                Some("merge") => "merge",
                Some("reply") => "reply",
                _ => "spawn",
            }
        }
        Criterion::WatchPasses { .. } | Criterion::CheckPasses { .. } => "check",
        Criterion::Judge { .. } => "judge",
    }
}

fn step_title(step: &Step) -> &str {
    step.task.lines().next().unwrap_or_default().trim()
}

fn brief_rows(task: &str, width: usize) -> Vec<Row> {
    let wrap_at = BRIEF_WIDTH.min(width.saturating_sub(BRIEF_INDENT.len()));
    task.lines()
        .flat_map(|line| wrap(line, wrap_at))
        .map(|line| match line.is_empty() {
            true => Row::new(Role::Brief, ""),
            false => Row::new(Role::Brief, format!("{BRIEF_INDENT}{line}")),
        })
        .collect()
}

fn step_rows(index: usize, step: &Step, width: usize) -> Vec<Row> {
    let mut rows = vec![Row::new(
        Role::Header,
        format!(
            "  {}. {} · {}",
            index + 1,
            step_kind(step),
            step_title(step)
        ),
    )];
    rows.extend(brief_rows(&step.task, width));
    rows.push(Row::new(
        Role::Exit,
        format!("     exit: {}", step.criterion.label()),
    ));
    if step.criterion.runs_in_ralph() {
        rows.push(Row::new(Role::Dim, format!("     {RUN_BY_RALPH_NOTE}")));
    }
    rows
}

fn denied_line(allowed: &[Action]) -> String {
    let denied: Vec<&str> = Action::KINDS
        .iter()
        .filter(|kind| {
            !allowed
                .iter()
                .any(|action| std::mem::discriminant(action) == std::mem::discriminant(*kind))
        })
        .map(Action::kind_label)
        .collect();
    if denied.is_empty() {
        "Not allowed: none".to_string()
    } else {
        format!("Not allowed: {}", denied.join(", "))
    }
}

fn caps_line(plan: &RunPlan) -> String {
    let caps = format!(
        "Caps: {} turns, ${:.2}",
        plan.limits.max_turns, plan.limits.max_usd
    );
    let judged = plan
        .steps
        .iter()
        .any(|step| matches!(step.criterion, Criterion::Judge { .. }));
    if judged {
        format!("{caps}, {} judge rounds", plan.judge_rounds)
    } else {
        caps
    }
}

fn plan_rows(plan: &RunPlan, width: usize) -> Vec<Row> {
    let mut rows = vec![
        Row::new(Role::Plain, format!("Goal: {}", plan.goal)),
        Row::new(Role::Plain, "Steps:"),
    ];
    for (index, step) in plan.steps.iter().enumerate() {
        rows.extend(step_rows(index, step, width));
    }
    let allowed: Vec<String> = plan.allowed.iter().map(Action::label).collect();
    rows.push(Row::new(
        Role::Plain,
        if allowed.is_empty() {
            "Allowed: none".to_string()
        } else {
            format!("Allowed: {}", allowed.join(", "))
        },
    ));
    rows.push(Row::new(Role::Dim, denied_line(&plan.allowed)));
    rows.extend(
        plan_warnings(plan)
            .into_iter()
            .map(|warning| Row::new(Role::Warning, warning)),
    );
    rows.push(Row::new(Role::Plain, caps_line(plan)));
    if !plan.grants.is_empty() {
        rows.push(Row::new(Role::Plain, "You pre-approve:"));
        rows.extend(
            plan.grants
                .iter()
                .map(|grant| Row::new(Role::Plain, format!("  - {}", grant.label()))),
        );
    }
    rows
}

fn state_row(file: &RunFile) -> Row {
    let hash = plan_hash(&file.plan);
    if file.is_approved() {
        Row::new(Role::Footer(GREEN), format!("{hash} · approved"))
    } else if let Some(step) = resume_step(file) {
        Row::new(
            Role::Footer(YELLOW),
            format!(
                "{hash} · stopped at step {step}/{} · /send-it go to resume from step {step}",
                file.plan.steps.len()
            ),
        )
    } else {
        Row::new(
            Role::Footer(YELLOW),
            format!("{hash} · not approved · /send-it go to approve"),
        )
    }
}

fn status_rows(file: &RunFile, width: usize) -> Vec<Row> {
    let mut rows = plan_rows(&file.plan, width);
    rows.push(state_row(file));
    rows
}

fn physical(row: &Row, width: usize) -> Vec<String> {
    match row.role {
        Role::Brief => vec![row.text.clone()],
        _ => hang_wrapped(&row.text, width)
            .split('\n')
            .map(str::to_string)
            .collect(),
    }
}

fn code_spans(text: &str, open: &mut bool) -> String {
    let mut out = String::new();
    if *open {
        out.push_str(CODE);
    }
    for c in text.chars() {
        match (c, *open) {
            ('`', true) => {
                out.push(c);
                out.push_str(RESET);
                *open = false;
            }
            ('`', false) => {
                out.push_str(CODE);
                out.push(c);
                *open = true;
            }
            _ => out.push(c),
        }
    }
    if *open {
        out.push_str(RESET);
    }
    out
}

fn paint_header(line: &str, first: bool) -> String {
    let parts = line
        .split_once(". ")
        .filter(|_| first)
        .and_then(|(lead, rest)| Some((lead, rest.split_once(" · ")?)));
    match parts {
        Some((lead, (kind, title))) => {
            format!("{lead}. {CODE}{kind}{RESET} · {BOLD}{title}{RESET}")
        }
        None => format!("{BOLD}{line}{RESET}"),
    }
}

fn paint_exit(line: &str, first: bool, open: &mut bool) -> String {
    match line.split_once("exit:").filter(|_| first) {
        Some((lead, rest)) => format!("{lead}{DIM}exit:{RESET}{}", code_spans(rest, open)),
        None => code_spans(line, open),
    }
}

fn paint_footer(line: &str, first: bool, tone: &str) -> String {
    match line.split_once(' ').filter(|_| first) {
        Some((hash, rest)) => format!("{DIM}{hash}{RESET} {tone}{rest}{RESET}"),
        None => format!("{tone}{line}{RESET}"),
    }
}

fn paint(role: Role, line: &str, first: bool, open: &mut bool) -> String {
    match role {
        Role::Plain | Role::Brief => line.to_string(),
        Role::Header => paint_header(line, first),
        Role::Exit => paint_exit(line, first, open),
        Role::Dim => format!("{DIM}{line}{RESET}"),
        Role::Warning => format!("{YELLOW}{line}{RESET}"),
        Role::Footer(tone) => paint_footer(line, first, tone),
    }
}

fn plain(rows: &[Row], width: usize) -> String {
    rows.iter()
        .flat_map(|row| physical(row, width))
        .collect::<Vec<_>>()
        .join("\n")
}

fn styled(rows: &[Row], width: usize) -> String {
    rows.iter()
        .flat_map(|row| {
            let mut open = false;
            physical(row, width)
                .into_iter()
                .enumerate()
                .map(|(index, line)| paint(row.role, &line, index == 0, &mut open))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_run(plan: &RunPlan) -> String {
    plain(&plan_rows(plan, NO_LIMIT), NO_LIMIT)
}

pub fn render_status(file: &RunFile, width: usize) -> String {
    plain(&status_rows(file, width), width)
}

pub fn render_status_styled(file: &RunFile, width: usize) -> String {
    styled(&status_rows(file, width), width)
}
