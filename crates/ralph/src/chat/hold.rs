use crate::chat::monitor::Monitor;
use crate::chat::prompt::screen_columns;
use crate::chat::statusbar::StatusBar;
use crate::chat::terminal::{poll_input, read_input, Input, PartialMode, POLL_MS};
use crate::signal;
use crate::warn::warn;
use ralph_core::chat_hold::{first_match, hold_key, redraw, HoldFilter};
use ralph_core::chat_status::Status;
use ralph_core::inbox::Position;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Held {
    Item(Position),
    Left,
    Quit,
}

fn matching_item(monitor: &Monitor, filter: &HoldFilter) -> io::Result<Option<Position>> {
    let ledger = monitor.inbox.ledger()?;
    Ok(first_match(ledger.entries(), filter))
}

fn show(bar: &StatusBar, status: &Status) {
    let mut out = io::stdout();
    let _ = write!(out, "{}", redraw(&bar.line(status, screen_columns())));
    let _ = out.flush();
}

struct LineEnd;

impl Drop for LineEnd {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = writeln!(out);
        let _ = out.flush();
    }
}

pub(super) fn run(filter: &HoldFilter, monitor: &Monitor, bar: &StatusBar) -> Held {
    let _terminal = io::stdin().is_terminal().then(PartialMode::enter).flatten();
    let outcome = {
        let _line = LineEnd;
        wait(filter, monitor, bar)
    };
    match outcome {
        Ok(held) => held,
        Err(error) => {
            warn(format!("/hold cannot read the inbox ledger: {error}"));
            Held::Left
        }
    }
}

fn wait(filter: &HoldFilter, monitor: &Monitor, bar: &StatusBar) -> io::Result<Held> {
    let interrupt = signal::interrupt_flag();
    interrupt.store(false, Ordering::SeqCst);
    let mut shown: Option<Status> = None;
    loop {
        let mut status = monitor.status();
        status.hold = Some(filter.clone());
        if shown.as_ref() != Some(&status) {
            show(bar, &status);
            shown = Some(status);
        }
        if signal::is_terminated() {
            return Ok(Held::Quit);
        }
        if interrupt.swap(false, Ordering::SeqCst) {
            return Ok(Held::Left);
        }
        if let Some(position) = matching_item(monitor, filter)? {
            return Ok(Held::Item(position));
        }
        match poll_input(POLL_MS) {
            Input::Closed => return Ok(Held::Quit),
            Input::Idle => {}
            Input::Ready => {
                let bytes = read_input();
                if bytes.is_empty() {
                    thread::sleep(Duration::from_millis(POLL_MS as u64));
                } else if hold_key(&bytes).is_some() {
                    return Ok(Held::Left);
                }
            }
        }
    }
}
