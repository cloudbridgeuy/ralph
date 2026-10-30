//! `/workers`: the live workers as a table or a list, built from data the
//! shell collected.

use crate::chat_memory::{Entry, Role};
use crate::inbox::age;
use crate::worker::{branch_name, ActivityMark, Report, StatusLine, Ticket};
use crate::worker_check::{is_report_status, message_header};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::HashMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod detail;
mod remote;

pub use detail::detail_view;
pub use remote::{
    fetched_of, lookup_command, ticket_lookup, wanted_keys, Fetched, LookupKey, Pr, PrState,
    Remote, RemoteCache, Review, TicketRemote,
};
use remote::{pr_cell, ticket_cell};

pub const REPORT_LIMIT: usize = 5;
pub const NO_WORKERS: &str = "No live workers.";
const MIN_SUBJECT: usize = 20;
const MIN_COLUMN: usize = 3;
const COLUMNS: usize = 9;
const HEADERS: [&str; COLUMNS] = [
    "ID", "HARNESS", "STATE", "REPORT", "HEAD", "+/-", "BEHIND", "TICKET", "PR",
];
pub const TONE_DONE: char = '\u{206A}';
pub const TONE_BLOCKED: char = '\u{206B}';
pub const TONE_WAITING: char = '\u{206C}';
pub const TONE_ACTIVE: char = '\u{206D}';
pub const TONE_END: char = '\u{206E}';
pub const TONE_MARKERS: [char; 5] = [TONE_DONE, TONE_BLOCKED, TONE_WAITING, TONE_ACTIVE, TONE_END];
const SUBJECT_SEPARATOR: &str = "¦";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportLine {
    pub status: String,
    pub at: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup<T> {
    Missing,
    Failed,
    Found(T),
}

impl<T> Lookup<T> {
    fn spans(&self, show: impl Fn(&T) -> Vec<Span>) -> Vec<Span> {
        match self {
            Lookup::Missing => vec![dim("-")],
            Lookup::Failed => vec![dim("?")],
            Lookup::Found(value) => show(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Name,
    Dim,
    Sha,
    Added,
    Removed,
    Behind,
    Done,
    Blocked,
    Attention,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub tone: Tone,
    pub text: String,
}

fn span(tone: Tone, text: impl Into<String>) -> Span {
    Span {
        tone,
        text: text.into(),
    }
}

fn plain(text: impl Into<String>) -> Span {
    span(Tone::Plain, text)
}

fn dim(text: impl Into<String>) -> Span {
    span(Tone::Dim, text)
}

fn cell_text(text: impl Into<String>) -> Vec<Span> {
    let text = text.into();
    let tone = if matches!(text.as_str(), "-" | "?" | "…") {
        Tone::Dim
    } else {
        Tone::Plain
    };
    vec![span(tone, text)]
}

pub fn spans_text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

pub fn lines_text(lines: &[Vec<Span>]) -> String {
    lines
        .iter()
        .map(|line| spans_text(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_tone(state: &str) -> Tone {
    match state {
        "done" => Tone::Done,
        "blocked" => Tone::Blocked,
        "question" | "permission" => Tone::Attention,
        "running" | "busy" => Tone::Active,
        "idle" => Tone::Dim,
        _ => Tone::Plain,
    }
}

fn state_span(row: &WorkerRow) -> Span {
    match &row.state {
        Some(state) => span(status_tone(state), state.as_str()),
        None => dim("-"),
    }
}

fn behind_span(count: usize, text: String) -> Span {
    let tone = if count > 0 { Tone::Behind } else { Tone::Dim };
    span(tone, text)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub hash: String,
    pub subject: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    pub added: usize,
    pub deleted: usize,
    pub files: usize,
    pub untracked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerRow {
    pub id: String,
    pub harness: String,
    pub state: Option<String>,
    pub report: Lookup<ReportLine>,
    pub status: Option<StatusLine>,
    pub head: Lookup<Head>,
    pub diff: Lookup<Diff>,
    pub behind: Lookup<usize>,
    pub base: Option<String>,
    pub ticket: Option<TicketRemote>,
    pub pr: Remote<Pr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerDetail {
    pub row: WorkerRow,
    pub branch: String,
    pub worktree: Option<String>,
    pub tmux: Option<String>,
    pub markers: Vec<&'static str>,
    pub heartbeat: Option<ActivityMark>,
    pub ticket: Option<Ticket>,
    pub history: Vec<StatusLine>,
    pub reports: Vec<ReportLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Table(String),
    Text(Vec<Vec<Span>>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Table,
    List,
}

fn entry_reports(text: &str, at: &str) -> Vec<(String, ReportLine)> {
    let mut found: Vec<(String, ReportLine)> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        match message_header(line) {
            Some((id, status)) => {
                open = is_report_status(status);
                if open {
                    found.push((
                        id.to_string(),
                        ReportLine {
                            status: status.to_string(),
                            at: at.to_string(),
                            text: String::new(),
                        },
                    ));
                }
            }
            None if open => {
                if let Some((_, report)) = found.last_mut() {
                    report.text.push_str(line);
                    report.text.push('\n');
                }
            }
            None => {}
        }
    }
    for (_, report) in &mut found {
        report.text = report.text.trim().to_string();
    }
    found
}

fn instant(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

pub fn group_reports(entries: &[Entry], unread: &[Report]) -> HashMap<String, Vec<ReportLine>> {
    let logged = entries
        .iter()
        .filter(|entry| entry.role == Role::Worker)
        .flat_map(|entry| entry_reports(&entry.text, &entry.at));
    let waiting = unread
        .iter()
        .filter(|report| is_report_status(&report.status))
        .map(|report| {
            (
                report.worker.clone(),
                ReportLine {
                    status: report.status.clone(),
                    at: report.at.clone(),
                    text: report.text.clone(),
                },
            )
        });
    let mut grouped: HashMap<String, Vec<ReportLine>> = HashMap::new();
    for (id, report) in logged.chain(waiting) {
        grouped.entry(id).or_default().push(report);
    }
    for reports in grouped.values_mut() {
        reports.reverse();
        reports.sort_by_key(|report| Reverse(instant(&report.at)));
    }
    grouped
}

pub fn parse_numstat(output: &str) -> Diff {
    let mut diff = Diff::default();
    for line in output.lines() {
        let mut fields = line.splitn(3, '\t');
        if let (Some(added), Some(deleted), Some(_)) = (fields.next(), fields.next(), fields.next())
        {
            diff.files += 1;
            diff.added += added.parse().unwrap_or(0);
            diff.deleted += deleted.parse().unwrap_or(0);
        }
    }
    diff
}

pub fn count_untracked(porcelain: &str) -> usize {
    porcelain
        .lines()
        .filter(|line| line.starts_with("?? "))
        .count()
}

pub fn parse_head(line: &str) -> Option<Head> {
    let line = line.trim();
    let (hash, subject) = line.split_once(' ').unwrap_or((line, ""));
    (!hash.is_empty()).then(|| Head {
        hash: hash.to_string(),
        subject: subject.trim().to_string(),
    })
}

pub fn not_live(id: &str) -> String {
    format!("No live worker {id}. /workers lists them.")
}

pub fn cut(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let room = width.saturating_sub(1);
    let mut used = 0;
    let mut out: String = text
        .chars()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= room
        })
        .collect();
    if width > 0 {
        out.push('…');
    }
    out
}

fn plural(count: usize, noun: &str) -> String {
    let s = if count == 1 { "" } else { "s" };
    format!("{count} {noun}{s}")
}

fn cut_spans(spans: Vec<Span>, width: usize) -> Vec<Span> {
    let text = spans_text(&spans);
    if text.width() <= width {
        return spans;
    }
    let mut left = cut(&text, width).chars().count() - usize::from(width > 0);
    let mut out: Vec<Span> = spans
        .into_iter()
        .filter_map(|span| {
            let kept: String = span.text.chars().take(left).collect();
            left -= kept.chars().count();
            (!kept.is_empty()).then_some(Span { text: kept, ..span })
        })
        .collect();
    if width > 0 {
        out.push(dim("…"));
    }
    out
}

fn report_cell(report: &ReportLine, now: DateTime<Utc>) -> Vec<Span> {
    vec![
        span(status_tone(&report.status), report.status.as_str()),
        plain(" "),
        dim(format!("{} ago", age(&report.at, now))),
    ]
}

fn status_cell(status: &StatusLine, now: DateTime<Utc>, text_limit: usize) -> Vec<Span> {
    vec![
        plain(format!("{} · ", cut(&status.text, text_limit))),
        dim(format!("{} ago", age(&status.at, now))),
    ]
}

fn is_newer(status_at: &str, report_at: &str) -> bool {
    match (
        DateTime::parse_from_rfc3339(status_at),
        DateTime::parse_from_rfc3339(report_at),
    ) {
        (Ok(status), Ok(report)) => status > report,
        _ => false,
    }
}

fn shown_status(row: &WorkerRow) -> Option<&StatusLine> {
    let status = row.status.as_ref()?;
    match &row.report {
        Lookup::Found(report) if !is_newer(&status.at, &report.at) => None,
        _ => Some(status),
    }
}

fn activity_cell(row: &WorkerRow, now: DateTime<Utc>, text_limit: usize) -> Vec<Span> {
    match shown_status(row) {
        Some(status) => status_cell(status, now, text_limit),
        None => row.report.spans(|report| report_cell(report, now)),
    }
}

fn head_cell(head: &Head, subject_limit: usize) -> Vec<Span> {
    let subject = cut(&head.subject, subject_limit);
    let mut cell = vec![span(Tone::Sha, head.hash.as_str())];
    if !subject.is_empty() {
        cell.push(plain(format!(" {subject}")));
    }
    cell
}

fn untracked_suffix(diff: &Diff) -> String {
    if diff.untracked == 0 {
        String::new()
    } else {
        format!(" ?{}", diff.untracked)
    }
}

fn diff_spans(diff: &Diff, tail: String) -> Vec<Span> {
    vec![
        span(Tone::Added, format!("+{}", diff.added)),
        plain(" "),
        span(Tone::Removed, format!("-{}", diff.deleted)),
        plain(tail),
    ]
}

fn diff_cell(diff: &Diff) -> Vec<Span> {
    diff_spans(diff, format!(" {}f{}", diff.files, untracked_suffix(diff)))
}

fn diff_text(diff: &Diff) -> Vec<Span> {
    diff_spans(
        diff,
        format!(
            " in {}{}",
            plural(diff.files, "file"),
            untracked_suffix(diff)
        ),
    )
}

fn behind_text(row: &WorkerRow) -> Vec<Span> {
    match (&row.behind, &row.base) {
        (Lookup::Found(count), Some(base)) => {
            vec![behind_span(*count, format!("{count} behind origin/{base}"))]
        }
        (Lookup::Found(count), None) => vec![behind_span(*count, format!("{count} behind"))],
        (other, _) => other.spans(|count| vec![behind_span(*count, count.to_string())]),
    }
}

fn table_cells(row: &WorkerRow, now: DateTime<Utc>, subject_limit: usize) -> [Vec<Span>; COLUMNS] {
    [
        vec![span(Tone::Name, row.id.as_str())],
        vec![dim(row.harness.as_str())],
        vec![state_span(row)],
        activity_cell(row, now, subject_limit),
        row.head.spans(|head| head_cell(head, subject_limit)),
        row.diff.spans(diff_cell),
        row.behind
            .spans(|count| vec![behind_span(*count, count.to_string())]),
        cell_text(ticket_cell(&row.id, &row.ticket)),
        cell_text(pr_cell(&row.pr)),
    ]
    .map(|cell| {
        cell.into_iter()
            .map(|span| Span {
                text: span
                    .text
                    .replace('|', SUBJECT_SEPARATOR)
                    .replace(TONE_MARKERS, ""),
                ..span
            })
            .collect()
    })
}

fn table_width(cells: &[[Vec<Span>; COLUMNS]]) -> usize {
    let columns: usize = (0..HEADERS.len())
        .map(|i| {
            cells
                .iter()
                .map(|row| spans_text(&row[i]).width())
                .chain([HEADERS[i].width(), MIN_COLUMN])
                .max()
                .unwrap_or(0)
        })
        .sum();
    columns + HEADERS.len() + 1
}

fn toned(start: char, text: &str) -> String {
    format!("{start}{text}{TONE_END}")
}

fn marked(span: &Span) -> String {
    match span.tone {
        Tone::Done | Tone::Added => toned(TONE_DONE, &span.text),
        Tone::Blocked | Tone::Removed => toned(TONE_BLOCKED, &span.text),
        Tone::Attention | Tone::Behind => toned(TONE_WAITING, &span.text),
        Tone::Active => toned(TONE_ACTIVE, &span.text),
        Tone::Name => format!("**{}**", span.text),
        Tone::Sha => format!("`{}`", span.text),
        Tone::Dim => format!("*{}*", span.text),
        _ => span.text.clone(),
    }
}

fn cell_markdown(cell: &[Span], styled: bool) -> String {
    let text = spans_text(cell);
    match (styled, text.as_str()) {
        (true, _) => cell.iter().map(marked).collect(),
        (false, "-") => "`-`".to_string(),
        _ => text,
    }
}

fn markdown(cells: &[[Vec<Span>; COLUMNS]], styled: bool) -> String {
    let line = |row: &[&str]| format!("| {} |", row.join(" | "));
    let separator = vec!["---"; HEADERS.len()];
    let mut lines = vec![line(&HEADERS), line(&separator)];
    lines.extend(cells.iter().map(|row| {
        let row: Vec<String> = row.iter().map(|cell| cell_markdown(cell, styled)).collect();
        line(&row.iter().map(String::as_str).collect::<Vec<_>>())
    }));
    lines.join("\n")
}

fn table(rows: &[WorkerRow], now: DateTime<Utc>, width: usize, styled: bool) -> Option<String> {
    let build = |limit| -> Vec<[Vec<Span>; COLUMNS]> {
        rows.iter()
            .map(|row| table_cells(row, now, limit))
            .collect()
    };
    let longest = rows
        .iter()
        .flat_map(|row| {
            let subject = match &row.head {
                Lookup::Found(head) => Some(head.subject.width()),
                _ => None,
            };
            let status = shown_status(row).map(|status| status.text.width());
            [subject, status]
        })
        .flatten()
        .max()
        .unwrap_or(0);
    let full = build(longest);
    let total = table_width(&full);
    if total <= width {
        return Some(markdown(&full, styled));
    }
    let limit = longest.saturating_sub(total - width).max(MIN_SUBJECT);
    let cut_cells = build(limit);
    (table_width(&cut_cells) <= width).then(|| markdown(&cut_cells, styled))
}

fn row_lines(row: &WorkerRow, now: DateTime<Utc>) -> Vec<(&'static str, Vec<Span>)> {
    let mut diff = row.diff.spans(diff_text);
    diff.push(plain(" · "));
    diff.extend(behind_text(row));
    vec![
        ("report", activity_cell(row, now, usize::MAX)),
        ("head", row.head.spans(|head| head_cell(head, usize::MAX))),
        ("diff", diff),
        ("ticket", cell_text(ticket_cell(&row.id, &row.ticket))),
        ("pr", cell_text(pr_cell(&row.pr))),
    ]
}

fn block(title: Vec<Span>, lines: &[(&str, Vec<Span>)], width: usize) -> Vec<Vec<Span>> {
    let label = lines
        .iter()
        .map(|(name, _)| name.width())
        .max()
        .unwrap_or(0);
    let mut out = vec![cut_spans(title, width)];
    out.extend(lines.iter().map(|(name, value)| {
        let mut line = vec![plain("  "), dim(format!("{name:<label$}")), plain("  ")];
        line.extend(value.iter().cloned());
        cut_spans(line, width)
    }));
    out
}

fn title(row: &WorkerRow) -> Vec<Span> {
    let state = state_span(row);
    vec![
        span(state.tone, "●"),
        plain(" "),
        span(Tone::Name, row.id.as_str()),
        plain(" · "),
        dim(row.harness.as_str()),
        plain(" · "),
        state,
    ]
}

pub fn workers_text(rows: &[WorkerRow], now: DateTime<Utc>) -> String {
    if rows.is_empty() {
        return "no workers".to_string();
    }
    rows.iter()
        .map(|row| {
            let (kind, since) = match &row.report {
                Lookup::Found(report) => (report.status.clone(), age(&report.at, now)),
                Lookup::Missing => ("-".to_string(), "-".to_string()),
                Lookup::Failed => ("?".to_string(), "?".to_string()),
            };
            format!(
                "{} {} {} {kind} {since} {}",
                row.id,
                row.harness,
                row.state.as_deref().unwrap_or("-"),
                branch_name(&row.id)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn workers_view(
    rows: &[WorkerRow],
    now: DateTime<Utc>,
    width: usize,
    layout: Layout,
    styled: bool,
) -> View {
    if rows.is_empty() {
        return View::Text(vec![vec![plain(NO_WORKERS)]]);
    }
    if layout == Layout::Table {
        if let Some(markdown) = table(rows, now, width, styled) {
            return View::Table(markdown);
        }
    }
    View::Text(
        rows.iter()
            .map(|row| block(title(row), &row_lines(row, now), width))
            .collect::<Vec<_>>()
            .join(&Vec::new()),
    )
}

#[cfg(test)]
mod tests;
