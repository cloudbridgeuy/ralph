use crate::frame::worker_header;
use crate::markdown::MarkdownTheme;
use crate::render::{render_markdown, RenderContext};
use crate::ui_theme::UiColors;
use ralph_core::chat_status::INBOX_USAGE;
use ralph_core::inbox::{self, Position, Selection, UnreadTurn};
use ralph_core::worker::Report;
use ralph_server::worker_mcp::Inbox;
use std::io::{self, Write};

pub struct Display<'a> {
    pub ctx: RenderContext<'a>,
    pub theme: &'a MarkdownTheme,
    pub color: bool,
    pub ui: UiColors,
}

pub fn read(
    selection: Selection,
    reports: &[Report],
    display: &Display,
    width: usize,
    out: &mut dyn Write,
) -> io::Result<Option<UnreadTurn>> {
    match inbox::unread_turn(selection, reports) {
        Err(message) => {
            writeln!(out, "{message}")?;
            Ok(None)
        }
        Ok(turn) => {
            let shown: Vec<String> = reports
                .iter()
                .map(|report| render_report(report, display, width))
                .collect();
            write!(out, "{}", shown.join("\n\n"))?;
            Ok(Some(turn))
        }
    }
}

pub fn list(inbox: &Inbox, out: &mut dyn Write) -> io::Result<()> {
    let ledger = inbox.ledger()?;
    writeln!(out, "{}", inbox::list(ledger.entries(), chrono::Utc::now()))
}

pub fn peek(
    inbox: &Inbox,
    position: Position,
    display: &Display,
    width: usize,
    out: &mut dyn Write,
) -> io::Result<()> {
    match inbox.ledger()?.get(position) {
        Some(e) => writeln!(out, "{}", render_report(&e.report, display, width)),
        None => writeln!(out, "No report {position}. /inbox list shows the queue."),
    }
}

pub fn drop_report(inbox: &Inbox, position: Position) -> io::Result<String> {
    Ok(match inbox.drop_report(position)? {
        Some(_) => format!("Dropped report {position}."),
        None => format!("No report {position}. /inbox list shows the queue."),
    })
}

pub fn usage(out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "{INBOX_USAGE}")
}

fn render_report(report: &Report, display: &Display, width: usize) -> String {
    format!(
        "{}\n{}",
        worker_header(
            &report.worker,
            report.status,
            width,
            display.color.then_some(&display.theme.palette)
        ),
        render_markdown(&display.ctx, &report.text, display.theme)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;

    fn report(worker: &str) -> Report {
        Report::from_worker(
            worker,
            "2026-11-17T00:00:00Z",
            ralph_core::worker::ReportStatus::Done,
            format!("{worker} finished"),
        )
    }

    fn with_display<R>(check: impl FnOnce(&Display) -> R) -> R {
        let highlighter = Highlighter::new();
        let theme = MarkdownTheme::default();
        check(&Display {
            ctx: RenderContext::new(&highlighter, false),
            theme: &theme,
            color: false,
            ui: UiColors::default(),
        })
    }

    #[test]
    fn read_with_no_reports_writes_the_refusal_line() {
        let mut out = Vec::new();
        let turn = with_display(|display| read(Selection::Next, &[], display, 40, &mut out));
        assert!(turn.unwrap().is_none());
        assert_eq!(String::from_utf8(out).unwrap(), "No unread reports.\n");
    }

    #[test]
    fn read_writes_each_report_at_the_given_width() {
        let reports = [report("w1"), report("w2")];
        let mut out = Vec::new();
        let (turn, shown) = with_display(|display| {
            let turn = read(Selection::All, &reports, display, 40, &mut out);
            let shown: Vec<String> = reports
                .iter()
                .map(|r| render_report(r, display, 40))
                .collect();
            (turn, shown.join("\n\n"))
        });
        assert!(turn.unwrap().is_some());
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, shown);
        let header = worker_header(
            "w1",
            ralph_core::worker::ReportKind::Worker(ralph_core::worker::ReportStatus::Done),
            40,
            None,
        );
        assert!(out.starts_with(&format!("{header}\n")), "{out}");
    }

    #[test]
    fn usage_writes_the_usage_line() {
        let mut out = Vec::new();
        usage(&mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), format!("{INBOX_USAGE}\n"));
    }
}
