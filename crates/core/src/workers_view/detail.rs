use super::{block, cut, row_lines, title, WorkerDetail, WorkerRow, REPORT_LIMIT};
use crate::inbox::age;
use chrono::{DateTime, Utc};

pub fn detail_view(detail: &WorkerDetail, now: DateTime<Utc>, width: usize) -> String {
    let row = &detail.row;
    let mut lines = row_lines(
        &WorkerRow {
            status: None,
            ..row.clone()
        },
        now,
    );
    lines.push(("branch", detail.branch.clone()));
    lines.push((
        "worktree",
        detail.worktree.clone().unwrap_or_else(|| "-".to_string()),
    ));
    lines.push(("base", row.base.clone().unwrap_or_else(|| "-".to_string())));
    if let Some(session) = &detail.tmux {
        lines.push(("tmux", session.clone()));
    }
    if !detail.markers.is_empty() {
        lines.push(("markers", detail.markers.join(", ")));
    }
    if let Some(mark) = &detail.heartbeat {
        let at = DateTime::from_timestamp(i64::try_from(mark.at).unwrap_or(i64::MAX), 0)
            .map(|at| at.to_rfc3339())
            .unwrap_or_default();
        let state = if mark.running { "running" } else { "idle" };
        lines.push(("heartbeat", format!("{state} {}", age(&at, now))));
    }
    let mut out = block(title(row), &lines, width);
    out.push_str("\n\n");
    if let Some(status) = &row.status {
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
    use super::super::tests::{line, now, row, with_status};
    use super::super::{ActivityMark, WorkerDetail};
    use super::*;
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
            reports: vec![],
        };
        let text = detail_view(&detail, now(), 40);
        assert!(!text.contains("tmux") && !text.contains("markers") && !text.contains("heartbeat"));
        assert!(text.contains("worktree  -"));
        assert!(text.ends_with("\n\nNo reports."));
        assert!(text.lines().all(|l| l.width() <= 40), "{text}");
    }

    #[test]
    fn detail_puts_the_status_line_above_the_reports() {
        let detail = WorkerDetail {
            row: with_status(row("GUZ-1")),
            branch: "worker/GUZ-1".into(),
            worktree: None,
            tmux: None,
            markers: vec![],
            heartbeat: None,
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
            row: with_status(row("GUZ-1")),
            branch: "worker/GUZ-1".into(),
            worktree: None,
            tmux: None,
            markers: vec![],
            heartbeat: None,
            reports: vec![],
        };
        let text = detail_view(&detail, now(), 200);
        assert!(
            text.ends_with("\n\nstatus: Reading the tests. (5m ago)\nNo reports."),
            "{text}"
        );
    }
}
