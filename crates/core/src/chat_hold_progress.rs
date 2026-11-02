use crate::ansi::{DIM, RESET};
use crate::chat_hold::{redraw as hold_redraw, HoldFilter};
use crate::schedule_worker::source_matches_worker;
use crate::worker::ActivityState;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const SEPARATOR: &str = " · ";
const ELLIPSIS: &str = "…";
const MAX_LINES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldProgress {
    pub id: String,
    pub elapsed_secs: u64,
    pub activity: Option<ActivityState>,
    pub rebase: Option<(u32, u32)>,
    pub commits: Option<u32>,
    pub hint: Option<String>,
}

fn elapsed_label(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
    }
}

fn activity_word(state: ActivityState) -> &'static str {
    match state {
        ActivityState::Running | ActivityState::Background => "busy",
        ActivityState::Idle => "idle",
        ActivityState::Failed => "failed",
    }
}

fn commits_label(count: u32) -> String {
    let noun = if count == 1 { "commit" } else { "commits" };
    format!("{count} {noun}")
}

fn fixed_segments(progress: &HoldProgress) -> Vec<String> {
    [
        Some(elapsed_label(progress.elapsed_secs)),
        progress
            .activity
            .map(|state| activity_word(state).to_string()),
        progress
            .rebase
            .map(|(done, total)| format!("rebase {done}/{total}")),
        progress.commits.map(commits_label),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn truncate_to_width(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .collect()
}

fn ellipsize(text: &str, width: usize) -> Option<String> {
    if text.width() <= width {
        return Some(text.to_string());
    }
    let room = width.checked_sub(ELLIPSIS.width())?;
    let kept = truncate_to_width(text, room);
    (!kept.is_empty()).then(|| format!("{kept}{ELLIPSIS}"))
}

fn plain_line(progress: &HoldProgress, width: usize) -> String {
    let fixed = fixed_segments(progress);
    let mut parts = vec![progress.id.clone()];
    parts.extend(fixed.iter().cloned());
    while parts.len() > 1 {
        let joined = parts.join(SEPARATOR);
        if joined.width() <= width {
            return match &progress.hint {
                Some(hint) if parts.len() == fixed.len() + 1 => {
                    let room = width.saturating_sub(joined.width() + SEPARATOR.width());
                    match ellipsize(hint, room) {
                        Some(shown) => format!("{joined}{SEPARATOR}{shown}"),
                        None => joined,
                    }
                }
                _ => joined,
            };
        }
        parts.pop();
    }
    ellipsize(&progress.id, width).unwrap_or_else(|| ELLIPSIS.repeat(usize::from(width > 0)))
}

fn styled(text: String, color: bool) -> String {
    if color && !text.is_empty() {
        format!("{DIM}{text}{RESET}")
    } else {
        text
    }
}

pub fn line(progress: &HoldProgress, width: usize, color: bool) -> String {
    styled(plain_line(progress, width), color)
}

pub fn rebase_progress(msgnum: &str, end: &str) -> Option<(u32, u32)> {
    let done = msgnum.trim().parse().ok()?;
    let total = end.trim().parse().ok().filter(|total| *total > 0)?;
    Some((done, total))
}

pub fn last_hint(pane: &str) -> Option<String> {
    pane.lines()
        .rev()
        .map(|line| line.chars().filter(|c| !c.is_control()).collect::<String>())
        .map(|line| line.trim().to_string())
        .find(|line| !line.is_empty())
}

pub fn held_ids(filter: &HoldFilter, running: &[String]) -> Vec<String> {
    let sources: &[String] = match filter {
        HoldFilter::Any => return running.to_vec(),
        HoldFilter::Source(source) => std::slice::from_ref(source),
        HoldFilter::Set(sources) => sources,
    };
    running
        .iter()
        .filter(|id| {
            sources
                .iter()
                .any(|source| source_matches_worker(source, id))
        })
        .cloned()
        .collect()
}

pub fn lines(progress: &[HoldProgress], width: usize, color: bool) -> Vec<String> {
    let mut shown: Vec<String> = progress
        .iter()
        .take(MAX_LINES)
        .map(|item| line(item, width, color))
        .collect();
    let hidden = progress.len().saturating_sub(MAX_LINES);
    if hidden > 0 {
        shown.push(styled(
            truncate_to_width(&format!("+{hidden} more"), width),
            color,
        ));
    }
    shown
}

pub fn redraw(
    lines: &[String],
    previous_lines: usize,
    bar: &str,
    painted: usize,
    columns: usize,
) -> String {
    let climb = match previous_lines {
        0 => String::new(),
        rows => format!("\x1b[{rows}A"),
    };
    let above: String = lines.iter().map(|line| format!("{line}\n")).collect();
    format!(
        "{climb}{}",
        hold_redraw(&format!("{above}{bar}"), painted, columns)
    )
}

#[cfg(test)]
mod tests;
