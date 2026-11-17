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

fn pending_rows(items: &[String]) -> Vec<Row> {
    if items.is_empty() {
        return vec![("Pending", "nothing".to_string())];
    }
    let texts = items
        .iter()
        .enumerate()
        .map(|(index, item)| format!("{}. {item} {MODEL_TAG}", index + 1))
        .collect();
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
        .chain(pending_rows(&driving.tally.pending))
        .chain([("Next", next)])
        .collect();
    render_block(&title, &rows, columns)
}

impl Ticked {
    pub fn shown_met(&self) -> Option<&str> {
        self.met.as_deref().filter(|_| self.next != Next::Done)
    }
}
