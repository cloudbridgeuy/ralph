use crate::chat::editor::open_in_editor;
use crate::chat::terminal::{poll_input, read_input, Input, POLL_MS};
use crate::signal;
use ralph_core::cancel::CancelToken;
use ralph_server::chat::ask::{Ask, Notices};
use ralph_server::chat::schedule::approve_proposed as approve;
use ralph_server::schedule_store::ScheduleStore;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub(super) fn read_answer() -> Option<String> {
    let cancel = CancelToken::default();
    let _armed = signal::arm(&cancel);
    loop {
        if signal::is_terminated() || cancel.is_cancelled() {
            return None;
        }
        match poll_input(POLL_MS) {
            Input::Closed => return None,
            Input::Idle => {}
            Input::Ready => {
                let bytes = read_input();
                return (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }
}

pub(super) struct Keyboard<'a> {
    pub editor: &'a [String],
}

impl Ask for Keyboard<'_> {
    fn answer(&mut self, prompt: &str) -> Option<String> {
        let mut out = io::stdout();
        say_raw(&mut out, prompt);
        let line = read_answer();
        if line.is_none() {
            say_raw(&mut out, "\n");
        }
        line
    }

    fn edit(&mut self, path: &Path) -> io::Result<()> {
        open_in_editor(self.editor, path)
    }
}

pub(super) fn approve_proposed(store: &ScheduleStore, default_timeout: Duration) -> bool {
    let notices: Notices = Arc::new(|line| println!("{line}"));
    let mut keyboard = Keyboard { editor: &[] };
    approve(store, default_timeout, &mut keyboard, &notices)
}

fn say_raw(out: &mut impl Write, text: &str) {
    let _ = write!(out, "{text}");
    let _ = out.flush();
}
