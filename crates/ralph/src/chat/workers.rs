use crate::ansi::{fg, BOLD, DIM, RESET};
use crate::chat::inbox::Display;
use crate::chat::memory::{load_usage, usage_path};
use crate::chat::monitor::Monitor;
use crate::chat::prompt::screen_columns;
use crate::chat::workers_remote::RemoteStatus;
use crate::config::{AppConfig, WorkersSection};
use crate::markdown::{MarkdownTheme, Palette};
use crate::render::render_markdown;
use crate::snapshot::{base_with, stdout_of, ORIGIN_HEAD_ARGS};
use crate::workers::running_workers;
use ralph_core::chat_status::RunningWorker;
use ralph_core::wire::price::PriceOverride;
use ralph_core::wire::provider::{provider, ProviderId};
use ralph_core::wire::view::Pricing;
use ralph_core::worker::{
    activity_path, blocked_path, branch_name, decode_spawn_record, decode_status, decode_statuses,
    gone_path, sent_path, spawn_record_path, status_path, unreported_path, waiting_path,
    worktree_for_branch, ActivityMark, HarnessId, StatusLine, Ticket,
};
use ralph_core::worker_costs::costs_view;
use ralph_core::workers_view::{
    count_untracked, detail_view, lines_text, not_live, parse_head, parse_numstat, workers_view,
    Diff, Lookup, ReportLine, Span, Tone, View, WorkerDetail, WorkerRow, TONE_ACTIVE, TONE_BLOCKED,
    TONE_DONE, TONE_END, TONE_WAITING,
};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::{Command, Stdio};
use termimad::crossterm::style::Attribute;
use termimad::CompoundStyle;

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
    workers: WorkersSection,
    origin_head: Option<String>,
    reports: Option<HashMap<String, Vec<ReportLine>>>,
    porcelain: Option<String>,
}

fn status_history(store: &Path, id: &str) -> Vec<StatusLine> {
    std::fs::read_to_string(status_path(store, id))
        .map(|raw| decode_statuses(&raw))
        .unwrap_or_default()
}

fn status_line(store: &Path, id: &str) -> Option<StatusLine> {
    decode_status(&std::fs::read_to_string(status_path(store, id)).ok()?)
}

fn spawn_ticket(store: &Path, id: &str) -> Option<Ticket> {
    decode_spawn_record(&std::fs::read_to_string(spawn_record_path(store, id)).ok()?)?.ticket()
}

fn row(
    monitor: &Monitor,
    worker: &RunningWorker,
    facts: &Facts,
    remote: &RemoteStatus,
    ticket: Option<&Ticket>,
) -> (WorkerRow, Option<String>) {
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
    let cells = remote.cells(&facts.workers, &worker.id, ticket);
    (
        WorkerRow {
            id: worker.id.clone(),
            harness: worker.kind.as_str().to_string(),
            state,
            report,
            status: status_line(&monitor.store_dir, &worker.id),
            head,
            diff,
            behind,
            base,
            ticket: cells.ticket,
            pr: cells.pr,
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
    let config = AppConfig::load().map_err(|e| e.to_string());
    Facts {
        workers: config
            .as_ref()
            .map(|config| config.workers.clone())
            .unwrap_or_default(),
        config,
        origin_head: git(None, &ORIGIN_HEAD_ARGS),
        reports,
        porcelain: git(None, &["worktree", "list", "--porcelain"]),
    }
}

fn wrapped(open: &str, text: &str) -> String {
    if open.is_empty() {
        text.to_string()
    } else {
        format!("{open}{text}{RESET}")
    }
}

fn paint_span(span: &Span, palette: &Palette) -> String {
    let open = match span.tone {
        Tone::Plain => String::new(),
        Tone::Name => BOLD.to_string(),
        Tone::Dim => DIM.to_string(),
        Tone::Done | Tone::Added => fg(palette.done),
        Tone::Blocked | Tone::Removed => fg(palette.blocked),
        Tone::Attention | Tone::Behind => fg(palette.waiting),
        Tone::Active => fg(palette.link),
        Tone::Sha => fg(palette.inline_code),
    };
    wrapped(&open, &span.text)
}

fn paint_lines(lines: &[Vec<Span>], palette: &Palette, color: bool) -> String {
    if !color {
        return lines_text(lines);
    }
    lines
        .iter()
        .map(|line| {
            line.iter()
                .map(|span| paint_span(span, palette))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn table_theme(theme: &MarkdownTheme) -> MarkdownTheme {
    let mut skin = theme.skin.clone();
    skin.italic = CompoundStyle::with_attr(Attribute::Dim);
    skin.inline_code.object_style.background_color = None;
    MarkdownTheme {
        skin,
        palette: theme.palette.clone(),
    }
}

const COLOR_OFF: &str = "\x1b[39m";

fn paint_tones(rendered: &str, palette: &Palette) -> String {
    let mut out = String::with_capacity(rendered.len());
    let mut open = false;
    for ch in rendered.chars() {
        let start = match ch {
            TONE_DONE => Some(palette.done),
            TONE_BLOCKED => Some(palette.blocked),
            TONE_WAITING => Some(palette.waiting),
            TONE_ACTIVE => Some(palette.link),
            _ => None,
        };
        match (ch, start) {
            (_, Some(color)) => {
                open = true;
                out.push_str(&fg(color));
            }
            (TONE_END, None) => {
                open = false;
                out.push_str(COLOR_OFF);
            }
            ('\n', None) if open => {
                open = false;
                out.push_str(COLOR_OFF);
                out.push('\n');
            }
            _ => out.push(ch),
        }
    }
    out
}

fn shown(view: &View, display: &Display) -> String {
    match view {
        View::Table(markdown) => paint_tones(
            &render_markdown(&display.ctx, markdown, &table_theme(display.theme)),
            &display.theme.palette,
        ),
        View::Text(lines) => format!(
            "{}\n",
            paint_lines(lines, &display.theme.palette, display.color)
        ),
    }
}

pub(super) fn costs(display: &Display, prices: &BTreeMap<String, PriceOverride>) {
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: prices,
    };
    match load_usage(&usage_path()) {
        Ok(usage) => print!(
            "{}",
            render_markdown(
                &display.ctx,
                &costs_view(&usage, &pricing),
                &table_theme(display.theme)
            )
        ),
        Err(error) => println!("Could not read the usage record: {error}"),
    }
}

pub(super) fn run(monitor: &Monitor, id: Option<&str>, display: &Display, remote: &RemoteStatus) {
    let running = running_workers(&monitor.store_dir);
    let facts = facts(monitor);
    let now = chrono::Utc::now();
    let width = screen_columns();
    let Some(id) = id else {
        let rows: Vec<WorkerRow> = running
            .iter()
            .map(|worker| {
                let ticket = spawn_ticket(&monitor.store_dir, &worker.id);
                row(monitor, worker, &facts, remote, ticket.as_ref()).0
            })
            .collect();
        print!(
            "{}",
            shown(
                &workers_view(&rows, now, width, facts.workers.view, display.color),
                display
            )
        );
        return;
    };
    let Some(worker) = running.iter().find(|worker| worker.id == id) else {
        println!("{}", not_live(id));
        return;
    };
    let history = status_history(&monitor.store_dir, id);
    let ticket = spawn_ticket(&monitor.store_dir, id);
    let (mut row, worktree) = row(monitor, worker, &facts, remote, ticket.as_ref());
    row.status = history.last().cloned();
    let detail = WorkerDetail {
        row,
        branch: branch_name(id),
        worktree,
        tmux: (worker.kind == HarnessId::Claude && !worker.session.is_empty())
            .then(|| worker.session.clone()),
        markers: markers(&monitor.store_dir, id),
        heartbeat: heartbeat(&monitor.store_dir, id),
        ticket,
        history,
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
    use ralph_core::workers_view::{Head, Pr, PrState, Remote, Review, TicketRemote};
    use unicode_width::UnicodeWidthStr;

    fn workers_view(rows: &[WorkerRow], now: DateTime<Utc>, width: usize) -> View {
        super::workers_view(
            rows,
            now,
            width,
            ralph_core::workers_view::Layout::Table,
            false,
        )
    }

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
            status: None,
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
            ticket: None,
            pr: Remote::None,
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
            View::Text(lines) => lines_text(lines),
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
        let cells = [
            7,
            7,
            5,
            12,
            "2f7f8c0 x".len(),
            "+120 -40 3f ?2".len(),
            6,
            6,
            3,
        ];
        assert_eq!(widest, cells.iter().sum::<usize>() + 9 + 1, "{text}");
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

    fn with_remote(mut row: WorkerRow) -> WorkerRow {
        row.ticket = Some(TicketRemote {
            key: row.id.clone(),
            state: Remote::Found("In Progress".into()),
        });
        row.pr = Remote::Found(Pr {
            number: 12,
            state: PrState::Open,
            draft: true,
            review: Review::Other,
            url: String::new(),
        });
        row
    }

    #[test]
    fn remote_cells_render_within_the_width_at_80_and_160_columns() {
        let rows = [
            with_remote(row(
                "GUZ-844",
                "fix(chat): a long subject for the head cell",
            )),
            row("GUZ-2", "feat: short"),
        ];
        let (view, wide) = rendered(&rows, 160);
        assert!(is_table(&view), "{wide}");
        assert!(wide.contains("TICKET") && wide.contains("PR"), "{wide}");
        assert!(
            wide.contains("In Progress") && wide.contains("#12 draft"),
            "{wide}"
        );
        let (view, narrow) = rendered(&rows, 80);
        assert!(!is_table(&view), "{narrow}");
        assert!(narrow.contains("ticket  In Progress"), "{narrow}");
        assert!(narrow.contains("pr      #12 draft"), "{narrow}");
        for (width, text) in [(160, &wide), (80, &narrow)] {
            for line in text.lines() {
                assert!(line.width() <= width, "{width} columns: {line}");
            }
        }
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

    #[test]
    fn the_ticket_is_read_from_the_spawn_record() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(spawn_ticket(dir.path(), "GUZ-1"), None);
        let path = spawn_record_path(dir.path(), "GUZ-1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"ticket":"GUZ-1","tracker":"linear","at":1}"#).unwrap();
        assert_eq!(
            spawn_ticket(dir.path(), "GUZ-1").map(|ticket| ticket.key),
            Some("GUZ-1".to_string())
        );
        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(spawn_ticket(dir.path(), "GUZ-1"), None);
    }

    fn shown_table(rows: &[WorkerRow], color: bool) -> String {
        let highlighter = crate::highlight::Highlighter::new();
        let mut ctx = crate::render::RenderContext::new(&highlighter, color);
        ctx.width = 200;
        let theme = MarkdownTheme::default();
        let display = Display {
            ctx,
            theme: &theme,
            color,
        };
        let view = super::workers_view(
            rows,
            now(),
            200,
            ralph_core::workers_view::Layout::Table,
            color,
        );
        shown(&view, &display)
    }

    #[test]
    fn colour_off_table_output_is_unchanged() {
        let out = shown_table(&[row("GUZ-1", "fix: x")], false);
        assert_eq!(out, concat!(
                "| ID | HARNESS | STATE | REPORT | HEAD | +/- | BEHIND | TICKET | PR |\n",
                "| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n",
                "| GUZ-1 | claude | busy | done 12m ago | 2f7f8c0 fix: x | +120 -40 3f ?2 | 2 | `-` | `-` |"
            ));
    }

    #[test]
    fn colour_on_tables_paint_diff_and_behind_cells() {
        let palette = MarkdownTheme::default().palette;
        let out = shown_table(&[row("GUZ-1", "fix: x")], true);
        assert!(
            out.contains(&format!("{}+120", fg(palette.done))),
            "{out:?}"
        );
        assert!(
            out.contains(&format!("{}-40", fg(palette.blocked))),
            "{out:?}"
        );
        assert!(
            out.contains(&format!("{}2", fg(palette.waiting))),
            "{out:?}"
        );
    }

    #[test]
    fn colour_on_tables_keep_equal_display_widths() {
        let rows = [
            row("GUZ-844", "fix(chat): exact /help and stricter tests"),
            row("GUZ-2", "feat: short"),
        ];
        let out = shown_table(&rows, true);
        let done = fg(MarkdownTheme::default().palette.done);
        assert!(out.contains(&format!("{done}+120")), "{out:?}");
        let plain: String = strip_ansi(&out)
            .chars()
            .filter(|ch| !('\u{206A}'..='\u{206E}').contains(ch))
            .collect();
        let widths: Vec<usize> = plain.lines().map(|line| line.width()).collect();
        assert!(widths.len() > 3, "{plain}");
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "{widths:?}\n{plain}"
        );
    }

    #[test]
    fn paint_tones_maps_each_marker_to_its_palette_slot() {
        let palette = MarkdownTheme::default().palette;
        let cases = [
            (TONE_DONE, palette.done),
            (TONE_BLOCKED, palette.blocked),
            (TONE_WAITING, palette.waiting),
            (TONE_ACTIVE, palette.link),
        ];
        for (marker, color) in cases {
            let text = format!("a {marker}x{TONE_END} b");
            assert_eq!(
                paint_tones(&text, &palette),
                format!("a {}x{COLOR_OFF} b", fg(color))
            );
        }
    }

    #[test]
    fn paint_tones_closes_an_open_tone_at_a_newline() {
        let palette = MarkdownTheme::default().palette;
        let text = format!("{TONE_DONE}a\nb{TONE_END}\nc");
        assert_eq!(
            paint_tones(&text, &palette),
            format!("{}a{COLOR_OFF}\nb{COLOR_OFF}\nc", fg(palette.done))
        );
    }

    #[test]
    fn paint_tones_leaves_text_without_markers_alone() {
        let palette = MarkdownTheme::default().palette;
        assert_eq!(paint_tones("a\n\x1b[1mb", &palette), "a\n\x1b[1mb");
    }
}
