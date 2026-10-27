use super::remote::{pr_cell, pr_detail, ticket_detail};
use super::{
    block, cell_text, cut, lines_text, row_lines, spans_text, title, Remote, WorkerDetail,
    WorkerRow, REPORT_LIMIT,
};
use crate::inbox::age;
use chrono::{DateTime, Utc};
use unicode_width::UnicodeWidthStr;

pub fn detail_view(detail: &WorkerDetail, now: DateTime<Utc>, width: usize) -> String {
    let row = &detail.row;
    let mut lines = row_lines(
        &WorkerRow {
            status: None,
            ..row.clone()
        },
        now,
    );
    let label = lines
        .iter()
        .map(|(name, _)| name.width())
        .max()
        .unwrap_or(0);
    let mut url_line = None;
    for (name, value) in &mut lines {
        match *name {
            "ticket" => *value = cell_text(ticket_detail(detail.ticket.as_ref(), &row.ticket)),
            "pr" => {
                *value = cell_text(pr_detail(&row.pr));
                if let Remote::Found(pr) = &row.pr {
                    if !pr.url.is_empty() && 2 + label + 2 + spans_text(value).width() > width {
                        *value = cell_text(pr_cell(&row.pr));
                        url_line = Some(format!("  {}", pr.url));
                    }
                }
            }
            _ => {}
        }
    }
    lines.push(("branch", cell_text(detail.branch.clone())));
    lines.push((
        "worktree",
        cell_text(detail.worktree.clone().unwrap_or_else(|| "-".to_string())),
    ));
    lines.push((
        "base",
        cell_text(row.base.clone().unwrap_or_else(|| "-".to_string())),
    ));
    if let Some(session) = &detail.tmux {
        lines.push(("tmux", cell_text(session.clone())));
    }
    if !detail.markers.is_empty() {
        lines.push(("markers", cell_text(detail.markers.join(", "))));
    }
    if let Some(mark) = &detail.heartbeat {
        let at = DateTime::from_timestamp(i64::try_from(mark.at).unwrap_or(i64::MAX), 0)
            .map(|at| at.to_rfc3339())
            .unwrap_or_default();
        let state = if mark.running { "running" } else { "idle" };
        lines.push(("heartbeat", cell_text(format!("{state} {}", age(&at, now)))));
    }
    let mut out = match url_line {
        None => lines_text(&block(title(row), &lines, width)),
        Some(url) => {
            let shown = lines_text(&block(title(row), &lines, width));
            let mut rows: Vec<&str> = shown.lines().collect();
            let at = rows
                .iter()
                .position(|line| line.trim_start().starts_with("pr "))
                .map_or(rows.len(), |i| i + 1);
            rows.insert(at, &url);
            rows.join("\n")
        }
    };
    out.push_str("\n\n");
    for status in detail.history.iter().rev() {
        out.push_str(&cut(
            &format!("status: {} ({} ago)", status.text, age(&status.at, now)),
            width,
        ));
        out.push('\n');
    }
    if detail.reports.is_empty() {
        out.push_str("No reports.");
    }
    out.push_str(
        &detail
            .reports
            .iter()
            .take(REPORT_LIMIT)
            .map(|report| {
                let first = report
                    .text
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("");
                cut(
                    &format!("{} {}  {first}", report.status, age(&report.at, now)),
                    width,
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::super::tests::{line, now, row};
    use super::super::{ActivityMark, Pr, PrState, Remote, Review, TicketRemote, WorkerDetail};
    use super::*;
    use crate::worker::{StatusLine, Ticket, Tracker};
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn detail_lists_extras_and_the_last_five_reports() {
        let reports: Vec<_> = (0..7)
            .map(|i| {
                line(
                    "done",
                    "2026-01-01T00:30:00Z",
                    &format!("\nreport {i}\nbody"),
                )
            })
            .collect();
        let detail = WorkerDetail {
            row: row("GUZ-1"),
            branch: "worker/GUZ-1".into(),
            worktree: Some("/w/GUZ-1".into()),
            tmux: Some("ralph-x-GUZ-1".into()),
            markers: vec!["sent", "gone"],
            ticket: None,
            history: vec![],
            heartbeat: Some(ActivityMark {
                at: 1_767_229_200 - 30,
                running: true,
            }),
            reports,
        };
        let text = detail_view(&detail, now(), 200);
        assert!(text.contains("  branch     worker/GUZ-1"), "{text}");
        assert!(text.contains("  worktree   /w/GUZ-1"));
        assert!(text.contains("  base       main"));
        assert!(text.contains("  tmux       ralph-x-GUZ-1"));
        assert!(text.contains("  markers    sent, gone"));
        assert!(text.contains("  heartbeat  running 30s"));
        let tail: Vec<_> = text.lines().skip_while(|l| !l.is_empty()).skip(1).collect();
        assert_eq!(tail.len(), 5);
        assert_eq!(tail[0], "done 30m  report 0");
        assert!(text.lines().all(|l| l.width() <= 200));
    }

    #[test]
    fn detail_omits_absent_extras_and_cuts_to_width() {
        let detail = WorkerDetail {
            row: row("GUZ-1"),
            branch: "worker/GUZ-1".into(),
            worktree: None,
            tmux: None,
            markers: vec![],
            heartbeat: None,
            ticket: None,
            history: vec![],
            reports: vec![],
        };
        let text = detail_view(&detail, now(), 40);
        assert!(!text.contains("tmux") && !text.contains("markers") && !text.contains("heartbeat"));
        assert!(text.contains("worktree  -"));
        assert!(text.ends_with("\n\nNo reports."));
        assert!(text.lines().all(|l| l.width() <= 40), "{text}");
    }

    fn reading() -> StatusLine {
        StatusLine {
            at: "2026-01-01T00:55:00Z".into(),
            text: "Reading the tests.".into(),
        }
    }

    #[test]
    fn detail_lists_the_status_history_newest_first_above_the_reports() {
        let older = StatusLine {
            at: "2026-01-01T00:40:00Z".into(),
            text: "Planning.".into(),
        };
        let oldest = StatusLine {
            at: "2026-01-01T00:10:00Z".into(),
            text: "Starting.".into(),
        };
        let detail = WorkerDetail {
            history: vec![oldest, older, reading()],
            reports: vec![line("done", "2026-01-01T00:30:00Z", "first")],
            ..bare_detail(None)
        };
        let text = detail_view(&detail, now(), 200);
        let tail: Vec<_> = text.lines().skip_while(|l| !l.is_empty()).skip(1).collect();
        assert_eq!(
            tail,
            [
                "status: Reading the tests. (5m ago)",
                "status: Planning. (20m ago)",
                "status: Starting. (50m ago)",
                "done 30m  first"
            ]
        );
    }

    #[test]
    fn detail_puts_the_status_line_above_the_reports() {
        let detail = WorkerDetail {
            row: row("GUZ-1"),
            branch: "worker/GUZ-1".into(),
            worktree: None,
            tmux: None,
            markers: vec![],
            heartbeat: None,
            ticket: None,
            history: vec![reading()],
            reports: vec![line("done", "2026-01-01T00:30:00Z", "first")],
        };
        let text = detail_view(&detail, now(), 200);
        let tail: Vec<_> = text.lines().skip_while(|l| !l.is_empty()).skip(1).collect();
        assert_eq!(
            tail,
            ["status: Reading the tests. (5m ago)", "done 30m  first"]
        );
        assert!(text.contains("  report    done 12m ago"), "{text}");
    }

    #[test]
    fn detail_with_status_and_no_reports_still_says_so() {
        let detail = WorkerDetail {
            row: row("GUZ-1"),
            branch: "worker/GUZ-1".into(),
            worktree: None,
            tmux: None,
            markers: vec![],
            heartbeat: None,
            ticket: None,
            history: vec![reading()],
            reports: vec![],
        };
        let text = detail_view(&detail, now(), 200);
        assert!(
            text.ends_with("\n\nstatus: Reading the tests. (5m ago)\nNo reports."),
            "{text}"
        );
    }

    fn bare_detail(ticket: Option<Ticket>) -> WorkerDetail {
        WorkerDetail {
            row: row("GUZ-1"),
            branch: "worker/GUZ-1".into(),
            worktree: None,
            tmux: None,
            markers: vec![],
            heartbeat: None,
            ticket,
            history: vec![],
            reports: vec![],
        }
    }

    #[test]
    fn detail_shows_the_ticket_with_its_tracker() {
        let ticket = Ticket {
            key: "GUZ-918".into(),
            tracker: Some(Tracker::Linear),
        };
        let text = detail_view(&bare_detail(Some(ticket)), now(), 200);
        assert!(text.contains("  ticket    GUZ-918 (linear)"), "{text}");
    }

    #[test]
    fn detail_shows_a_ticket_without_a_tracker_and_omits_an_absent_one() {
        let ticket = Ticket {
            key: "FOO-1".into(),
            tracker: None,
        };
        let text = detail_view(&bare_detail(Some(ticket)), now(), 200);
        assert!(text.contains("  ticket    FOO-1\n"), "{text}");
        let text = detail_view(&bare_detail(None), now(), 200);
        assert!(text.contains("  ticket    -\n"), "{text}");
    }

    fn remote_row(state: Remote<String>, pr: Remote<Pr>) -> WorkerRow {
        WorkerRow {
            ticket: Some(TicketRemote {
                key: "GUZ-846".into(),
                state,
            }),
            pr,
            ..row("GUZ-846")
        }
    }

    fn linear_ticket() -> Ticket {
        Ticket {
            key: "GUZ-846".into(),
            tracker: Some(Tracker::Linear),
        }
    }

    #[test]
    fn detail_shows_the_remote_state_and_the_pr_with_its_url() {
        let open = Pr {
            number: 12,
            state: PrState::Open,
            draft: false,
            review: Review::Other,
            url: "https://github.com/o/r/pull/12".into(),
        };
        let detail = WorkerDetail {
            row: remote_row(Remote::Found("In Progress".into()), Remote::Found(open)),
            ..bare_detail(Some(linear_ticket()))
        };
        let text = detail_view(&detail, now(), 200);
        assert!(
            text.contains("  ticket    GUZ-846 (linear) · In Progress\n"),
            "{text}"
        );
        assert!(
            text.contains("  pr        #12 open · https://github.com/o/r/pull/12\n"),
            "{text}"
        );
        assert!(text.lines().all(|l| l.width() <= 200), "{text}");
        let narrow = detail_view(&detail, now(), 80);
        assert!(narrow.lines().all(|l| l.width() <= 80), "{narrow}");
    }

    #[test]
    fn a_long_pr_url_is_never_cut_and_moves_to_its_own_line() {
        let url = format!("https://github.com/org/repo/pull/12/{}", "x".repeat(60));
        let open = Pr {
            number: 12,
            state: PrState::Open,
            draft: false,
            review: Review::Other,
            url: url.clone(),
        };
        let detail = WorkerDetail {
            row: remote_row(Remote::Found("In Progress".into()), Remote::Found(open)),
            ..bare_detail(Some(linear_ticket()))
        };
        let text = detail_view(&detail, now(), 80);
        let rows: Vec<&str> = text.lines().collect();
        let at = rows.iter().position(|l| l.starts_with("  pr ")).unwrap();
        assert_eq!(rows[at].trim_end(), "  pr        #12 open");
        assert_eq!(rows[at + 1], format!("  {url}"));
        let wide = detail_view(&detail, now(), 400);
        assert!(
            wide.contains(&format!("  pr        #12 open · {url}\n")),
            "{wide}"
        );
    }

    #[test]
    fn detail_shows_pending_and_failed_lookups() {
        let detail = WorkerDetail {
            row: remote_row(Remote::Pending, Remote::Failed),
            ..bare_detail(Some(linear_ticket()))
        };
        let text = detail_view(&detail, now(), 200);
        assert!(
            text.contains("  ticket    GUZ-846 (linear) · …\n"),
            "{text}"
        );
        assert!(text.contains("  pr        ?\n"), "{text}");
    }
}
