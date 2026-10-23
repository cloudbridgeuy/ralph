use crate::chat::inbox::Display;
use crate::chat::monitor::Monitor;
use crate::chat::prompt::screen_columns;
use crate::config::AppConfig;
use crate::render::render_markdown;
use crate::snapshot::{base_with, stdout_of, ORIGIN_HEAD_ARGS};
use crate::workers::running_workers;
use ralph_core::chat_status::RunningWorker;
use ralph_core::worker::{
    activity_path, blocked_path, branch_name, gone_path, sent_path, unreported_path, waiting_path,
    worktree_for_branch, ActivityMark, HarnessId,
};
use ralph_core::workers_view::{
    count_untracked, detail_view, not_live, parse_head, parse_numstat, workers_view, Diff, Lookup,
    ReportLine, View, WorkerDetail, WorkerRow,
};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

fn git(dir: Option<&Path>, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("--no-optional-locks");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    stdout_of(command.args(args).stdin(Stdio::null())).ok()
}

fn diff_of(worktree: &Path, base: &str) -> Option<Diff> {
    let merge_base = git(Some(worktree), &["merge-base", "HEAD", base])?;
    let numstat = git(Some(worktree), &["diff", "--numstat", &merge_base])?;
    let status = git(Some(worktree), &["status", "--porcelain", "-uall"])?;
    Some(Diff {
        untracked: count_untracked(&status),
        ..parse_numstat(&numstat)
    })
}

fn found<T>(value: Option<T>) -> Lookup<T> {
    value.map_or(Lookup::Failed, Lookup::Found)
}

struct Facts {
    config: Result<AppConfig, String>,
    origin_head: Option<String>,
    reports: Option<HashMap<String, Vec<ReportLine>>>,
    porcelain: Option<String>,
}

fn row(monitor: &Monitor, worker: &RunningWorker, facts: &Facts) -> (WorkerRow, Option<String>) {
    let state = monitor
        .status()
        .workers
        .into_iter()
        .find(|state| state.id == worker.id)
        .map(|state| state.state);
    let report = match &facts.reports {
        None => Lookup::Failed,
        Some(reports) => reports
            .get(&worker.id)
            .and_then(|list| list.first().cloned())
            .map_or(Lookup::Missing, Lookup::Found),
    };
    let tree = facts
        .porcelain
        .as_deref()
        .and_then(|porcelain| worktree_for_branch(porcelain, &branch_name(&worker.id)));
    let base = tree
        .as_ref()
        .map(|_| base_with(&facts.config, facts.origin_head.as_deref()));
    let (head, diff, behind) = match (&tree, &base) {
        (Some(tree), Some(base)) => (
            found(
                git(Some(tree), &["log", "-1", "--format=%h %s"])
                    .and_then(|line| parse_head(&line)),
            ),
            found(diff_of(tree, base)),
            found(
                git(
                    Some(tree),
                    &["rev-list", "--count", &format!("HEAD..origin/{base}")],
                )
                .and_then(|count| count.parse().ok()),
            ),
        ),
        _ if facts.porcelain.is_none() => (Lookup::Failed, Lookup::Failed, Lookup::Failed),
        _ => (Lookup::Missing, Lookup::Missing, Lookup::Missing),
    };
    let worktree = tree.map(|tree| tree.display().to_string());
    (
        WorkerRow {
            id: worker.id.clone(),
            harness: worker.kind.as_str().to_string(),
            state,
            report,
            head,
            diff,
            behind,
            base,
        },
        worktree,
    )
}

fn markers(store: &Path, id: &str) -> Vec<&'static str> {
    [
        ("blocked", blocked_path(store, id)),
        ("waiting", waiting_path(store, id)),
        ("unreported", unreported_path(store, id)),
        ("sent", sent_path(store, id)),
        ("gone", gone_path(store, id)),
    ]
    .into_iter()
    .filter(|(_, path)| path.exists())
    .map(|(name, _)| name)
    .collect()
}

fn heartbeat(store: &Path, id: &str) -> Option<ActivityMark> {
    ActivityMark::parse(&std::fs::read_to_string(activity_path(store, id)).ok()?)
}

fn facts(monitor: &Monitor) -> Facts {
    let reports = monitor.store.load().ok().and_then(|(entries, _)| {
        let unread = monitor.inbox.peek().ok()?;
        Some(ralph_core::workers_view::group_reports(&entries, &unread))
    });
    Facts {
        config: AppConfig::load().map_err(|e| e.to_string()),
        origin_head: git(None, &ORIGIN_HEAD_ARGS),
        reports,
        porcelain: git(None, &["worktree", "list", "--porcelain"]),
    }
}

pub(super) fn run(monitor: &Monitor, id: Option<&str>, display: &Display) {
    let running = running_workers(&monitor.store_dir);
    let facts = facts(monitor);
    let now = chrono::Utc::now();
    let width = screen_columns();
    let Some(id) = id else {
        let rows: Vec<WorkerRow> = running
            .iter()
            .map(|worker| row(monitor, worker, &facts).0)
            .collect();
        match workers_view(&rows, now, width) {
            View::Table(markdown) => {
                print!(
                    "{}",
                    render_markdown(&display.ctx, &markdown, display.theme)
                )
            }
            View::Text(text) => println!("{text}"),
        }
        return;
    };
    let Some(worker) = running.iter().find(|worker| worker.id == id) else {
        println!("{}", not_live(id));
        return;
    };
    let (row, worktree) = row(monitor, worker, &facts);
    let detail = WorkerDetail {
        row,
        branch: branch_name(id),
        worktree,
        tmux: (worker.kind == HarnessId::Claude && !worker.session.is_empty())
            .then(|| worker.session.clone()),
        markers: markers(&monitor.store_dir, id),
        heartbeat: heartbeat(&monitor.store_dir, id),
        reports: facts
            .reports
            .and_then(|mut reports| reports.remove(id))
            .unwrap_or_default(),
    };
    println!("{}", detail_view(&detail, now, width));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::strip_ansi;
    use crate::markdown::MarkdownTheme;
    use crate::render::text::render_prose;
    use chrono::{DateTime, Utc};
    use ralph_core::workers_view::Head;
    use unicode_width::UnicodeWidthStr;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn row(id: &str, subject: &str) -> WorkerRow {
        WorkerRow {
            id: id.into(),
            harness: "claude".into(),
            state: Some("busy".into()),
            report: Lookup::Found(ReportLine {
                status: "done".into(),
                at: "2026-01-01T00:48:00Z".into(),
                text: String::new(),
            }),
            head: Lookup::Found(Head {
                hash: "2f7f8c0".into(),
                subject: subject.into(),
            }),
            diff: Lookup::Found(Diff {
                added: 120,
                deleted: 40,
                files: 3,
                untracked: 2,
            }),
            behind: Lookup::Found(2),
            base: Some("main".into()),
        }
    }

    fn rendered(rows: &[WorkerRow], width: usize) -> (View, String) {
        let view = workers_view(rows, now(), width);
        let text = match &view {
            View::Table(markdown) => strip_ansi(&render_prose(
                &MarkdownTheme::default().skin,
                markdown,
                width,
            )),
            View::Text(text) => text.clone(),
        };
        (view, text)
    }

    fn is_table(view: &View) -> bool {
        matches!(view, View::Table(_))
    }

    #[test]
    fn termimad_tables_fit_the_estimated_width() {
        let short = [row("GUZ-844", "x"), row("GUZ-2", "y")];
        let (view, text) = rendered(&short, 160);
        assert!(is_table(&view), "{text}");
        let widest = text.lines().map(|line| line.width()).max().unwrap();
        let cells = [7, 7, 5, 12, "2f7f8c0 x".len(), "+120 -40 3f ?2".len(), 6];
        assert_eq!(widest, cells.iter().sum::<usize>() + 7 + 1, "{text}");
    }

    #[test]
    fn table_or_list_and_no_line_wider_than_the_width() {
        let long = [
            row(
                "GUZ-844",
                "fix(chat): exact /help, width-wrapped help and stricter help tests",
            ),
            row("GUZ-2", "feat: short"),
        ];
        for (width, table) in [(60, false), (100, true), (160, true)] {
            let (view, text) = rendered(&long, width);
            assert_eq!(is_table(&view), table, "{width} columns: {text}");
            for line in text.lines() {
                assert!(line.width() <= width, "{width} columns: {line}");
            }
        }
        let (_, cut) = rendered(&long, 100);
        assert!(cut.contains('…'), "{cut}");
        let (_, full) = rendered(&long, 160);
        assert!(full.contains("stricter help tests"), "{full}");
    }

    #[test]
    fn missing_and_failed_cells_draw_literally() {
        let mut gaps = row("GUZ-3", "z");
        gaps.report = Lookup::Missing;
        gaps.behind = Lookup::Missing;
        gaps.diff = Lookup::Failed;
        let (view, text) = rendered(&[gaps], 160);
        assert!(is_table(&view), "{text}");
        let drawn = text.lines().find(|l| l.contains("GUZ-3")).unwrap();
        let cells: Vec<&str> = drawn.split('│').map(str::trim).collect();
        assert_eq!(cells[4], "-", "{text}");
        assert_eq!(cells[7], "-", "{text}");
        assert_eq!(cells[6], "?", "{text}");
        assert!(!text.contains('•'), "{text}");
    }
}
