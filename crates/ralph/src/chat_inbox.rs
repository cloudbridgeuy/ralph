use crate::chat::screen_columns;
use crate::frame::worker_header;
use crate::markdown::MarkdownTheme;
use crate::render::{render_markdown, RenderContext};
use crate::worker_mcp::Inbox;
use ralph_core::chat_status::INBOX_USAGE;
use ralph_core::inbox::{self, Position, Selection, UnreadTurn};
use ralph_core::worker::Report;
use std::io;

pub struct Display<'a> {
    pub ctx: RenderContext<'a>,
    pub theme: &'a MarkdownTheme,
    pub color: bool,
}

pub fn read(selection: Selection, reports: &[Report], display: &Display) -> Option<UnreadTurn> {
    match inbox::unread_turn(selection, reports) {
        Err(message) => {
            println!("{message}");
            None
        }
        Ok(turn) => {
            let shown: Vec<String> = reports
                .iter()
                .map(|report| render_report(report, display))
                .collect();
            print!("{}", shown.join("\n\n"));
            Some(turn)
        }
    }
}

pub fn list(inbox: &Inbox) -> io::Result<()> {
    let ledger = inbox.ledger()?;
    println!("{}", inbox::list(ledger.entries(), chrono::Utc::now()));
    Ok(())
}

pub fn peek(inbox: &Inbox, position: Position, display: &Display) -> io::Result<()> {
    match inbox.ledger()?.get(position) {
        Some(e) => println!("{}", render_report(&e.report, display)),
        None => println!("No report {position}. /inbox list shows the queue."),
    }
    Ok(())
}

pub fn drop_report(inbox: &Inbox, position: Position) -> io::Result<()> {
    match inbox.drop_report(position)? {
        Some(_) => println!("Dropped report {position}."),
        None => println!("No report {position}. /inbox list shows the queue."),
    }
    Ok(())
}

pub fn usage() {
    println!("{INBOX_USAGE}");
}

fn render_report(report: &Report, display: &Display) -> String {
    format!(
        "{}\n{}",
        worker_header(
            &report.worker,
            &report.status,
            screen_columns(),
            display.color.then_some(&display.theme.palette)
        ),
        render_markdown(&display.ctx, &report.text, display.theme)
    )
}
