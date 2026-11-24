use crate::chat::inbox::Display;
use crate::chat::prompt::screen_columns;
use crate::chat::rerender::redraw_text;
use crate::turn_sink::ReplyWidth;
use ralph_core::chat_resize::{debounce, on_render, should_redraw, ResizeInput, DEBOUNCE_MS};
use ralph_server::watches::locked;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(50);

#[derive(Default)]
pub(super) struct Resize {
    rendered: Mutex<Option<usize>>,
    pending: AtomicBool,
}

impl ReplyWidth for Resize {
    fn begin(&self) {
        *locked(&self.rendered) = None;
    }

    fn rendered(&self, cols: usize) {
        let mut rendered = locked(&self.rendered);
        *rendered = Some(on_render(*rendered, cols));
    }
}

impl Resize {
    pub(super) fn take_pending(&self) -> bool {
        self.pending.swap(false, Ordering::SeqCst)
    }

    pub(super) fn mark_pending(&self) {
        self.pending.store(true, Ordering::SeqCst);
    }

    pub(super) fn take_redraw(&self, idle: bool, main_view: bool, new_cols: usize) -> bool {
        let mut rendered = locked(&self.rendered);
        let redraw = should_redraw(&ResizeInput {
            rendered: *rendered,
            new_cols,
            idle,
            main_view,
        });
        if redraw {
            *rendered = Some(new_cols);
        }
        redraw
    }
}

pub(super) struct Watch<'a> {
    pub(super) resized: &'a AtomicBool,
    pub(super) done: &'a AtomicBool,
    pub(super) at_prompt: &'a AtomicBool,
    pub(super) main_view: bool,
    pub(super) state: &'a Resize,
    pub(super) store: &'a Path,
    pub(super) display: &'a Display<'a>,
    pub(super) sink: SyncSender<String>,
}

impl Watch<'_> {
    pub(super) fn run(&self) {
        let started = Instant::now();
        let mut last = None;
        while !self.done.load(Ordering::SeqCst) {
            std::thread::sleep(TICK);
            if !self.resized.swap(false, Ordering::SeqCst) {
                continue;
            }
            let now = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            if !debounce(last, now, DEBOUNCE_MS) {
                continue;
            }
            last = Some(now);
            std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
            self.resized.store(false, Ordering::SeqCst);
            self.settle(screen_columns());
        }
    }

    fn settle(&self, cols: usize) {
        if !self.at_prompt.load(Ordering::SeqCst) {
            self.state.mark_pending();
            return;
        }
        if !self.state.take_redraw(true, self.main_view, cols) {
            return;
        }
        if let Some(text) = redraw_text(self.store, self.display, cols) {
            text.lines().for_each(|row| {
                self.sink.send(row.to_string()).ok();
            });
        }
    }
}
