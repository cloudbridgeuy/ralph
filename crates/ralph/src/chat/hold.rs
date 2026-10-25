use crate::chat::monitor::Monitor;
use crate::chat::prompt::screen_columns;
use crate::chat::statusbar::StatusBar;
use crate::chat::terminal::{poll_input, read_input, Input, PartialMode, POLL_MS};
use crate::signal;
use ralph_core::chat_hold::{first_match, hold_key, HoldFilter};
use ralph_core::chat_status::Status;
use ralph_core::inbox::Position;
use std::io::{self, IsTerminal};
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Held {
    Item(Position),
    Left,
    Quit,
}

fn matching_item(monitor: &Monitor, filter: &HoldFilter) -> Option<Position> {
    let ledger = monitor.inbox.ledger().ok()?;
    first_match(ledger.entries(), filter)
}

fn show(bar: &StatusBar, status: &Status) {
    print!("{}", bar.line(status, screen_columns()));
}

pub(super) fn run(filter: &HoldFilter, monitor: &Monitor, bar: &StatusBar) -> Held {
    let _terminal = io::stdin().is_terminal().then(PartialMode::enter).flatten();
    let interrupt = signal::interrupt_flag();
    let mut shown: Option<Status> = None;
    loop {
        let mut status = monitor.status();
        status.hold = Some(filter.clone());
        if shown.as_ref() != Some(&status) {
            show(bar, &status);
            shown = Some(status);
        }
        if signal::is_terminated() {
            return Held::Quit;
        }
        if interrupt.swap(false, Ordering::SeqCst) {
            return Held::Left;
        }
        if let Some(position) = matching_item(monitor, filter) {
            return Held::Item(position);
        }
        match poll_input(POLL_MS) {
            Input::Closed => return Held::Quit,
            Input::Idle => {}
            Input::Ready => {
                let bytes = read_input();
                if bytes.is_empty() {
                    thread::sleep(Duration::from_millis(POLL_MS as u64));
                } else if hold_key(&bytes).is_some() {
                    return Held::Left;
                }
            }
        }
    }
}
