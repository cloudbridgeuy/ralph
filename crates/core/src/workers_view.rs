//! `/workers`: the live workers as a table or a list, built from data the
//! shell collected.

use crate::chat_memory::{Entry, Role};
use crate::inbox::age;
use crate::worker::{ActivityMark, Report, StatusLine, Ticket};
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
    fn cell(&self, show: impl Fn(&T) -> String) -> String {
        match self {
            Lookup::Missing => "-".to_string(),
            Lookup::Failed => "?".to_string(),
            Lookup::Found(value) => show(value),
        }
    }
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
    Text(String),
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

fn report_cell(report: &ReportLine, now: DateTime<Utc>) -> String {
    format!("{} {} ago", report.status, age(&report.at, now))
}

fn status_cell(status: &StatusLine, now: DateTime<Utc>, text_limit: usize) -> String {
    format!(
        "{} · {} ago",
        cut(&status.text, text_limit),
        age(&status.at, now)
    )
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

fn activity_cell(row: &WorkerRow, now: DateTime<Utc>, text_limit: usize) -> String {
    match shown_status(row) {
        Some(status) => status_cell(status, now, text_limit),
        None => row.report.cell(|report| report_cell(report, now)),
    }
}

fn head_cell(head: &Head, subject_limit: usize) -> String {
    format!("{} {}", head.hash, cut(&head.subject, subject_limit))
        .trim_end()
        .to_string()
}

fn untracked_suffix(diff: &Diff) -> String {
    if diff.untracked == 0 {
        String::new()
    } else {
        format!(" ?{}", diff.untracked)
    }
}

fn diff_cell(diff: &Diff) -> String {
    format!(
        "+{} -{} {}f{}",
        diff.added,
        diff.deleted,
        diff.files,
        untracked_suffix(diff)
    )
}

fn diff_text(diff: &Diff) -> String {
    format!(
        "+{} -{} in {}{}",
        diff.added,
        diff.deleted,
        plural(diff.files, "file"),
        untracked_suffix(diff)
    )
}

fn behind_text(row: &WorkerRow) -> String {
    match (&row.behind, &row.base) {
        (Lookup::Found(count), Some(base)) => format!("{count} behind origin/{base}"),
        (Lookup::Found(count), None) => format!("{count} behind"),
        (other, _) => other.cell(usize::to_string),
    }
}

fn table_cells(row: &WorkerRow, now: DateTime<Utc>, subject_limit: usize) -> [String; COLUMNS] {
    [
        row.id.clone(),
        row.harness.clone(),
        row.state.clone().unwrap_or_else(|| "-".to_string()),
        activity_cell(row, now, subject_limit),
        row.head.cell(|head| head_cell(head, subject_limit)),
        row.diff.cell(diff_cell),
        row.behind.cell(usize::to_string),
        ticket_cell(&row.id, &row.ticket),
        pr_cell(&row.pr),
    ]
    .map(|cell| cell.replace('|', SUBJECT_SEPARATOR))
}

fn table_width(cells: &[[String; COLUMNS]]) -> usize {
    let columns: usize = (0..HEADERS.len())
        .map(|i| {
            cells
                .iter()
                .map(|row| row[i].width())
                .chain([HEADERS[i].width(), MIN_COLUMN])
                .max()
                .unwrap_or(0)
        })
        .sum();
    columns + HEADERS.len() + 1
}

fn markdown(cells: &[[String; COLUMNS]]) -> String {
    let line = |row: &[&str]| format!("| {} |", row.join(" | "));
    let separator = vec!["---"; HEADERS.len()];
    let mut lines = vec![line(&HEADERS), line(&separator)];
    lines.extend(cells.iter().map(|row| {
        let row: Vec<&str> = row
            .iter()
            .map(|cell| if cell == "-" { "`-`" } else { cell.as_str() })
            .collect();
        line(&row)
    }));
    lines.join("\n")
}

fn table(rows: &[WorkerRow], now: DateTime<Utc>, width: usize) -> Option<String> {
    let build = |limit| -> Vec<[String; COLUMNS]> {
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
        return Some(markdown(&full));
    }
    let limit = longest.saturating_sub(total - width).max(MIN_SUBJECT);
    let cut_cells = build(limit);
    (table_width(&cut_cells) <= width).then(|| markdown(&cut_cells))
}

fn row_lines(row: &WorkerRow, now: DateTime<Utc>) -> Vec<(&'static str, String)> {
    let diff = row.diff.cell(diff_text);
    vec![
        ("report", activity_cell(row, now, usize::MAX)),
        ("head", row.head.cell(|head| head_cell(head, usize::MAX))),
        ("diff", format!("{diff} · {}", behind_text(row))),
        ("ticket", ticket_cell(&row.id, &row.ticket)),
        ("pr", pr_cell(&row.pr)),
    ]
}

fn block(title: String, lines: &[(&str, String)], width: usize) -> String {
    let label = lines
        .iter()
        .map(|(name, _)| name.width())
        .max()
        .unwrap_or(0);
    let mut out = vec![cut(&title, width)];
    out.extend(
        lines
            .iter()
            .map(|(name, value)| cut(&format!("  {name:<label$}  {value}"), width)),
    );
    out.join("\n")
}

fn title(row: &WorkerRow) -> String {
    format!(
        "● {} · {} · {}",
        row.id,
        row.harness,
        row.state.as_deref().unwrap_or("-")
    )
}

pub fn workers_view(rows: &[WorkerRow], now: DateTime<Utc>, width: usize, layout: Layout) -> View {
    if rows.is_empty() {
        return View::Text(NO_WORKERS.to_string());
    }
    if layout == Layout::Table {
        if let Some(markdown) = table(rows, now, width) {
            return View::Table(markdown);
        }
    }
    View::Text(
        rows.iter()
            .map(|row| block(title(row), &row_lines(row, now), width))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    const NOW: &str = "2026-01-01T01:00:00Z";

    fn workers_view(rows: &[WorkerRow], now: DateTime<Utc>, width: usize) -> View {
        super::workers_view(rows, now, width, Layout::Table)
    }

    pub(super) fn now() -> DateTime<Utc> {
        instant(NOW).unwrap()
    }

    fn entry(role: Role, at: &str, text: &str) -> Entry {
        Entry {
            role,
            at: at.into(),
            text: text.into(),
            corrupt: false,
        }
    }

    pub(super) fn line(status: &str, at: &str, text: &str) -> ReportLine {
        ReportLine {
            status: status.into(),
            at: at.into(),
            text: text.into(),
        }
    }

    pub(super) fn row(id: &str) -> WorkerRow {
        WorkerRow {
            id: id.into(),
            harness: "claude".into(),
            state: Some("busy".into()),
            report: Lookup::Found(line("done", "2026-01-01T00:48:00Z", "")),
            status: None,
            head: Lookup::Found(Head {
                hash: "2f7f8c0".into(),
                subject: "fix(chat): exact /help, width-wrapped help and stricter help tests"
                    .into(),
            }),
            diff: Lookup::Found(Diff {
                added: 120,
                deleted: 40,
                files: 3,
                untracked: 2,
            }),
            behind: Lookup::Found(2),
            base: Some("main".into()),
            ticket: None,
            pr: Remote::None,
        }
    }

    #[test]
    fn several_reports_in_one_entry_are_split_newest_first() {
        let text = "[worker A · done]\nfirst\nmore\n\n[worker B · blocked]\nstuck\n\n[worker A · question]\nwhy?";
        let entries = [entry(Role::Worker, "2026-01-01T00:10:00Z", text)];
        let grouped = group_reports(&entries, &[]);
        assert_eq!(
            grouped["A"],
            vec![
                line("question", "2026-01-01T00:10:00Z", "why?"),
                line("done", "2026-01-01T00:10:00Z", "first\nmore"),
            ]
        );
        assert_eq!(grouped["B"].len(), 1);
    }

    #[test]
    fn non_report_statuses_and_other_roles_are_ignored() {
        let text = "[worker A · check]\nquiet\n\n[worker A · watch]\nexit 0";
        let entries = [
            entry(Role::Worker, "2026-01-01T00:10:00Z", text),
            entry(Role::User, "2026-01-01T00:11:00Z", "[worker A · done]\nx"),
        ];
        let unread = [Report {
            worker: "A".into(),
            at: "2026-01-01T00:12:00Z".into(),
            status: "check".into(),
            text: "quiet".into(),
        }];
        assert!(group_reports(&entries, &unread).is_empty());
    }

    #[test]
    fn a_check_header_ends_the_previous_report_text() {
        let text = "[worker A · done]\nreal\n\n[worker A · check]\nnoise";
        let entries = [entry(Role::Worker, "2026-01-01T00:10:00Z", text)];
        assert_eq!(group_reports(&entries, &[])["A"][0].text, "real");
    }

    #[test]
    fn unread_reports_sort_with_logged_ones_by_time() {
        let entries = [entry(
            Role::Worker,
            "2026-01-01T00:30:00Z",
            "[worker A · done]\nold",
        )];
        let unread = [Report {
            worker: "A".into(),
            at: "2026-01-01T00:40:00Z".into(),
            status: "blocked".into(),
            text: "new".into(),
        }];
        let statuses: Vec<_> = group_reports(&entries, &unread)["A"]
            .iter()
            .map(|r| r.status.clone())
            .collect();
        assert_eq!(statuses, ["blocked", "done"]);
    }

    #[test]
    fn numstat_counts_lines_and_files_with_binary_as_zero() {
        let diff = parse_numstat("10\t2\ta.rs\n-\t-\timg.png\n5\t0\tdir/b c.rs\n");
        assert_eq!(
            diff,
            Diff {
                added: 15,
                deleted: 2,
                files: 3,
                untracked: 0
            }
        );
        assert_eq!(parse_numstat(""), Diff::default());
    }

    #[test]
    fn untracked_counts_every_file_of_a_new_directory() {
        assert_eq!(count_untracked("?? d/a\n?? d/b\n?? d/c.rs\n M x"), 3);
    }

    #[test]
    fn untracked_counts_only_question_mark_lines() {
        assert_eq!(count_untracked("?? a\n M b\n?? c d\nA  e"), 2);
        assert_eq!(count_untracked(""), 0);
    }

    #[test]
    fn head_splits_hash_from_subject() {
        assert_eq!(
            parse_head("abc1234 fix: it\n"),
            Some(Head {
                hash: "abc1234".into(),
                subject: "fix: it".into()
            })
        );
        assert_eq!(parse_head("abc1234").unwrap().subject, "");
        assert_eq!(parse_head("  "), None);
    }

    #[test]
    fn cut_uses_display_width() {
        assert_eq!(cut("abcdef", 6), "abcdef");
        assert_eq!(cut("abcdef", 4), "abc…");
        assert_eq!(cut("漢字漢字", 5), "漢字…");
        assert_eq!(cut("abc", 0), "");
    }

    #[test]
    fn empty_workers_print_a_plain_line() {
        for layout in [Layout::Table, Layout::List] {
            assert_eq!(
                super::workers_view(&[], now(), 80, layout),
                View::Text(NO_WORKERS.to_string())
            );
        }
    }

    #[test]
    fn layout_selects_a_table_or_blocks_on_wide_screens() {
        let rows = [row("GUZ-1")];
        assert!(matches!(
            super::workers_view(&rows, now(), 160, Layout::Table),
            View::Table(_)
        ));
        assert_eq!(
            super::workers_view(&rows, now(), 160, Layout::List),
            View::Text("● GUZ-1 · claude · busy\n  report  done 12m ago\n  head    2f7f8c0 fix(chat): exact /help, width-wrapped help and stricter help tests\n  diff    +120 -40 in 3 files ?2 · 2 behind origin/main\n  ticket  -\n  pr      -".into())
        );
    }

    #[test]
    fn table_fallback_matches_list_layout_on_narrow_screens() {
        let rows = [row("GUZ-1")];
        let list = super::workers_view(&rows, now(), 80, Layout::List);
        assert!(matches!(list, View::Text(_)));
        assert_eq!(super::workers_view(&rows, now(), 80, Layout::Table), list);
    }

    fn table_of(view: View) -> String {
        match view {
            View::Table(markdown) => markdown,
            other => panic!("not a table: {other:?}"),
        }
    }

    #[test]
    fn wide_screens_get_a_table_with_the_full_subject() {
        let markdown = table_of(workers_view(&[row("GUZ-1")], now(), 160));
        assert_eq!(
            markdown,
            "| ID | HARNESS | STATE | REPORT | HEAD | +/- | BEHIND | TICKET | PR |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n| GUZ-1 | claude | busy | done 12m ago | 2f7f8c0 fix(chat): exact /help, width-wrapped help and stricter help tests | +120 -40 3f ?2 | 2 | `-` | `-` |"
        );
    }

    #[test]
    fn the_subject_is_cut_before_the_table_gives_up() {
        let markdown = table_of(workers_view(&[row("GUZ-1")], now(), 100));
        assert!(markdown.contains("…"), "{markdown}");
        assert!(table_width(&[table_cells(&row("GUZ-1"), now(), 20)]) <= 100);
    }

    #[test]
    fn narrow_screens_get_a_list_cut_to_width() {
        let View::Text(text) = workers_view(&[row("GUZ-1")], now(), 80) else {
            panic!("not a list");
        };
        assert_eq!(
            text,
            "● GUZ-1 · claude · busy\n  report  done 12m ago\n  head    2f7f8c0 fix(chat): exact /help, width-wrapped help and stricter help …\n  diff    +120 -40 in 3 files ?2 · 2 behind origin/main\n  ticket  -\n  pr      -"
        );
        for width in [10, 30, 80] {
            let View::Text(text) = workers_view(&[row("GUZ-1")], now(), width) else {
                panic!("not a list");
            };
            assert!(text.lines().all(|l| l.width() <= width), "{text}");
        }
    }

    #[test]
    fn missing_and_failed_values_show_dash_and_question_mark() {
        let mut gone = row("A");
        gone.state = None;
        gone.report = Lookup::Missing;
        gone.head = Lookup::Missing;
        gone.diff = Lookup::Failed;
        gone.behind = Lookup::Failed;
        let cells = table_cells(&gone, now(), 99);
        assert_eq!(cells[2], "-");
        assert_eq!(cells[3], "-");
        assert_eq!(cells[4], "-");
        assert_eq!(cells[5], "?");
        assert_eq!(cells[6], "?");
        let View::Text(text) = workers_view(&[gone], now(), 20) else {
            panic!("not a list");
        };
        assert!(text.contains("diff    ? · ?"), "{text}");
    }

    #[test]
    fn behind_text_names_the_remote_tracking_ref() {
        assert_eq!(behind_text(&row("A")), "2 behind origin/main");
        let mut bare = row("A");
        bare.base = None;
        assert_eq!(behind_text(&bare), "2 behind");
        bare.behind = Lookup::Failed;
        assert_eq!(behind_text(&bare), "?");
    }

    #[test]
    fn dash_cells_are_escaped_in_the_markdown() {
        let mut gone = row("A");
        gone.report = Lookup::Missing;
        gone.behind = Lookup::Missing;
        let View::Table(markdown) = workers_view(&[gone], now(), 400) else {
            panic!("not a table");
        };
        assert!(markdown.contains("| `-` | 2f7f8c0"), "{markdown}");
        assert!(markdown.ends_with("| `-` |"), "{markdown}");
    }

    #[test]
    fn untracked_count_is_omitted_at_zero() {
        let mut clean = row("A");
        clean.diff = Lookup::Found(Diff {
            added: 1,
            deleted: 0,
            files: 1,
            untracked: 0,
        });
        assert_eq!(table_cells(&clean, now(), 99)[5], "+1 -0 1f");
        assert!(row_lines(&clean, now())[2]
            .1
            .starts_with("+1 -0 in 1 file ·"));
    }

    #[test]
    fn pipes_in_a_subject_do_not_split_table_cells() {
        let mut piped = row("A");
        piped.head = Lookup::Found(Head {
            hash: "abc".into(),
            subject: "a | b".into(),
        });
        assert_eq!(table_cells(&piped, now(), 99)[4], "abc a ¦ b");
    }

    pub(super) fn with_status(mut row: WorkerRow) -> WorkerRow {
        row.status = Some(StatusLine {
            at: "2026-01-01T00:55:00Z".into(),
            text: "Reading the tests.".into(),
        });
        row
    }

    #[test]
    fn a_status_replaces_the_report_cell_with_text_and_age() {
        let cells = table_cells(&with_status(row("A")), now(), 99);
        assert_eq!(cells[3], "Reading the tests. · 5m ago");
        assert_eq!(
            row_lines(&with_status(row("A")), now())[0].1,
            "Reading the tests. · 5m ago"
        );
    }

    #[test]
    fn a_row_without_status_falls_back_to_the_latest_report() {
        assert_eq!(table_cells(&row("A"), now(), 99)[3], "done 12m ago");
        assert_eq!(row_lines(&row("A"), now())[0].1, "done 12m ago");
    }

    #[test]
    fn a_report_newer_than_the_status_is_shown_instead() {
        let mut newer = with_status(row("A"));
        newer.report = Lookup::Found(line("done", "2026-01-01T00:58:00Z", ""));
        assert_eq!(table_cells(&newer, now(), 99)[3], "done 2m ago");
        assert_eq!(row_lines(&newer, now())[0].1, "done 2m ago");
    }

    #[test]
    fn equal_times_and_unparsable_report_times_keep_the_report() {
        let mut same = with_status(row("A"));
        same.report = Lookup::Found(line("done", "2026-01-01T00:55:00Z", ""));
        assert_eq!(table_cells(&same, now(), 99)[3], "done 5m ago");
        same.report = Lookup::Found(line("done", "garbage", ""));
        assert!(!is_newer("2026-01-01T00:55:00Z", "garbage"));
        assert_eq!(shown_status(&same), None);
    }

    #[test]
    fn times_compare_across_offsets() {
        assert!(is_newer(
            "2026-01-01T00:55:00Z",
            "2026-01-01T01:50:00+01:00"
        ));
        assert!(!is_newer(
            "2026-01-01T00:55:00Z",
            "2026-01-01T00:40:00-01:00"
        ));
    }

    #[test]
    fn a_status_shows_when_there_is_no_report_to_compare() {
        for report in [Lookup::Missing, Lookup::Failed] {
            let mut r = with_status(row("A"));
            r.report = report;
            assert_eq!(table_cells(&r, now(), 99)[3], "Reading the tests. · 5m ago");
        }
    }

    #[test]
    fn a_long_status_is_cut_and_the_table_stays_a_table() {
        let mut long = with_status(row("GUZ-1"));
        let text = "s".repeat(120);
        long.status = Some(StatusLine {
            at: "2026-01-01T00:55:00Z".into(),
            text: text.clone(),
        });
        let markdown = table_of(workers_view(&[long.clone()], now(), 140));
        assert!(markdown.contains("…"), "{markdown}");
        assert!(!markdown.contains(&text));
        assert!(markdown.lines().all(|l| l.width() <= 140), "{markdown}");
        let View::Text(list) = workers_view(&[long], now(), 40) else {
            panic!("not a list");
        };
        assert!(!list.contains(&text));
    }

    #[test]
    fn a_wide_screen_keeps_a_long_status_whole() {
        let mut long = with_status(row("GUZ-1"));
        let text = "s".repeat(120);
        long.status = Some(StatusLine {
            at: "2026-01-01T00:55:00Z".into(),
            text: text.clone(),
        });
        assert!(table_of(workers_view(&[long], now(), 400)).contains(&text));
    }

    fn with_remote(mut row: WorkerRow) -> WorkerRow {
        row.ticket = Some(TicketRemote {
            key: row.id.clone(),
            state: Remote::Found("In Progress".into()),
        });
        row.pr = Remote::Found(Pr {
            number: 12,
            state: PrState::Open,
            draft: false,
            review: Review::Approved,
            url: "https://github.com/o/r/pull/12".into(),
        });
        row
    }

    #[test]
    fn the_table_at_160_columns_has_the_ticket_and_pr_cells() {
        let markdown = table_of(workers_view(&[with_remote(row("GUZ-1"))], now(), 160));
        let last = markdown.lines().last().unwrap();
        assert!(
            last.ends_with("| 2 | In Progress | #12 approved |"),
            "{markdown}"
        );
        assert!(markdown.contains("stricter help tests"), "{markdown}");
    }

    #[test]
    fn at_130_columns_the_subject_is_cut_before_the_new_columns() {
        let markdown = table_of(workers_view(&[with_remote(row("GUZ-1"))], now(), 130));
        assert!(markdown.contains('…'), "{markdown}");
        assert!(
            markdown.contains("| In Progress | #12 approved |"),
            "{markdown}"
        );
    }

    #[test]
    fn the_list_at_80_columns_has_ticket_and_pr_lines_within_the_width() {
        let View::Text(text) = workers_view(&[with_remote(row("GUZ-1"))], now(), 80) else {
            panic!("not a list");
        };
        assert!(text.contains("\n  ticket  In Progress\n"), "{text}");
        assert!(text.ends_with("\n  pr      #12 approved"), "{text}");
        assert!(text.lines().all(|l| l.width() <= 80), "{text}");
    }

    #[test]
    fn pending_and_failed_remote_cells_show_in_the_row() {
        let mut waiting = row("GUZ-1");
        waiting.ticket = Some(TicketRemote {
            key: "GUZ-1".into(),
            state: Remote::Pending,
        });
        waiting.pr = Remote::Failed;
        let cells = table_cells(&waiting, now(), 99);
        assert_eq!(cells[7], "…");
        assert_eq!(cells[8], "?");
    }

    #[test]
    fn not_live_names_the_id() {
        assert_eq!(not_live("X-1"), "No live worker X-1. /workers lists them.");
    }
}
