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
