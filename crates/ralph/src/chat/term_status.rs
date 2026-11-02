use crate::inbox_alert::{write_terminal, TMUX_PANE_ENV};
use ralph_core::program_status::{encode, tmux_wrap, Report, State};
use ralph_core::term_title::{title_end, title_start};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, PartialEq, Eq)]
struct Sent {
    last: Option<String>,
    closed: bool,
    title_open: bool,
}

fn frame(report: &Report, in_tmux: bool) -> Option<String> {
    let sequence = encode(report).ok()?;
    Some(if in_tmux {
        tmux_wrap(&sequence)
    } else {
        sequence
    })
}

fn changed(sent: &Sent, bytes: &str) -> bool {
    !sent.closed && sent.last.as_deref() != Some(bytes)
}

fn take_title_restore(sent: &mut Sent) -> bool {
    std::mem::take(&mut sent.title_open)
}

pub(super) struct TermStatus {
    enabled: bool,
    title_enabled: bool,
    in_tmux: bool,
    sent: Mutex<Sent>,
}

impl TermStatus {
    pub(super) fn new(enabled: bool, title_enabled: bool) -> Self {
        Self {
            enabled,
            title_enabled,
            in_tmux: std::env::var_os(TMUX_PANE_ENV).is_some(),
            sent: Mutex::new(Sent::default()),
        }
    }

    pub(super) fn send(&self, report: &Report) {
        if !self.enabled {
            return;
        }
        let Some(bytes) = frame(report, self.in_tmux) else {
            return;
        };
        let mut sent = crate::watches::locked(&self.sent);
        if changed(&sent, &bytes) {
            let _ = write_terminal(&bytes);
            sent.last = Some(bytes);
        }
    }

    pub(super) fn start_title(&self, name: &str) {
        if !self.title_enabled {
            return;
        }
        let mut sent = crate::watches::locked(&self.sent);
        if !sent.title_open {
            let _ = write_terminal(&title_start(name));
            sent.title_open = true;
        }
    }

    pub(super) fn clear(&self) {
        let mut sent = crate::watches::locked(&self.sent);
        if take_title_restore(&mut sent) {
            let _ = write_terminal(title_end());
        }
        if self.enabled && !sent.closed {
            let clear = Report {
                state: State::Clear,
                msg: None,
            };
            if let Some(bytes) = frame(&clear, self.in_tmux) {
                let _ = write_terminal(&bytes);
            }
            sent.closed = true;
        }
    }
}

pub(super) struct ClearOnDrop(pub(super) Arc<TermStatus>);

impl Drop for ClearOnDrop {
    fn drop(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::program_status::State;

    fn working() -> Report {
        Report {
            state: State::Working,
            msg: None,
        }
    }

    #[test]
    fn frame_is_the_plain_sequence_outside_tmux() {
        assert_eq!(
            frame(&working(), false).as_deref(),
            Some("\x1b]7501;state=working:app=ralph\x07")
        );
    }

    #[test]
    fn frame_wraps_the_sequence_inside_tmux() {
        assert_eq!(
            frame(&working(), true).as_deref(),
            Some("\x1bPtmux;\x1b\x1b]7501;state=working:app=ralph\x07\x1b\\")
        );
    }

    #[test]
    fn identical_bytes_are_not_resent() {
        let sent = Sent {
            last: Some("a".into()),
            ..Sent::default()
        };
        assert!(!changed(&sent, "a"));
        assert!(changed(&sent, "b"));
        assert!(changed(&Sent::default(), "a"));
    }

    #[test]
    fn title_restore_is_taken_at_most_once() {
        let mut sent = Sent {
            title_open: true,
            ..Sent::default()
        };
        assert!(take_title_restore(&mut sent));
        assert!(!take_title_restore(&mut sent));
        assert!(!take_title_restore(&mut Sent::default()));
    }

    #[test]
    fn nothing_is_written_after_the_status_is_closed() {
        let sent = Sent {
            closed: true,
            ..Sent::default()
        };
        assert!(!changed(&sent, "a"));
    }
}
