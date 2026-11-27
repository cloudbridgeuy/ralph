use std::io;

pub const LINE_CAP: usize = 8 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    Retry,
    Stop,
}

pub fn read_outcome(kind: io::ErrorKind) -> ReadOutcome {
    match kind {
        io::ErrorKind::Interrupted => ReadOutcome::Retry,
        _ => ReadOutcome::Stop,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReadEvent {
    Data,
    Eof,
    Failed(io::ErrorKind),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Process,
    Retry,
    Backoff,
    Hold,
    End,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    pub action: Action,
    pub note: bool,
}

pub struct ReaderState {
    failures: u32,
    noted: bool,
    max_failures: u32,
}

impl ReaderState {
    pub fn new(max_failures: u32) -> Self {
        ReaderState {
            failures: 0,
            noted: false,
            max_failures,
        }
    }

    pub fn next(&mut self, event: ReadEvent) -> Step {
        let (action, note) = match event {
            ReadEvent::Data => {
                self.failures = 0;
                (Action::Process, false)
            }
            ReadEvent::Eof => (Action::End, false),
            ReadEvent::Failed(kind) => match read_outcome(kind) {
                ReadOutcome::Retry => (Action::Retry, false),
                ReadOutcome::Stop => {
                    self.failures = self.failures.saturating_add(1);
                    let note = !self.noted;
                    self.noted = true;
                    if self.failures > self.max_failures {
                        (Action::Hold, note)
                    } else {
                        (Action::Backoff, note)
                    }
                }
            },
        };
        Step { action, note }
    }
}

pub struct LineSplitter {
    line: Vec<u8>,
    discarding: bool,
    cap: usize,
}

impl LineSplitter {
    pub fn new(cap: usize) -> Self {
        LineSplitter {
            line: Vec::new(),
            discarding: false,
            cap,
        }
    }

    pub fn push_bytes(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut done = Vec::new();
        for &byte in chunk {
            if byte == b'\n' {
                if self.line.last() == Some(&b'\r') {
                    self.line.pop();
                }
                done.push(String::from_utf8_lossy(&self.line).into_owned());
                self.line.clear();
                self.discarding = false;
            } else if !self.discarding {
                if self.line.len() < self.cap {
                    self.line.push(byte);
                } else {
                    self.discarding = true;
                }
            }
        }
        done
    }

    pub fn finish(self) -> Option<String> {
        if self.line.is_empty() {
            None
        } else {
            Some(String::from_utf8_lossy(&self.line).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(cap: usize, chunks: &[&[u8]]) -> Vec<String> {
        let mut splitter = LineSplitter::new(cap);
        let mut lines: Vec<String> = chunks
            .iter()
            .flat_map(|chunk| splitter.push_bytes(chunk))
            .collect();
        lines.extend(splitter.finish());
        lines
    }

    fn failed() -> ReadEvent {
        ReadEvent::Failed(io::ErrorKind::PermissionDenied)
    }

    #[test]
    fn data_is_processed() {
        let mut state = ReaderState::new(3);
        assert_eq!(
            state.next(ReadEvent::Data),
            Step {
                action: Action::Process,
                note: false
            }
        );
    }

    #[test]
    fn eof_ends_the_reader() {
        let mut state = ReaderState::new(3);
        assert_eq!(state.next(ReadEvent::Eof).action, Action::End);
    }

    #[test]
    fn an_interrupted_read_retries_without_a_note() {
        let mut state = ReaderState::new(3);
        assert_eq!(
            state.next(ReadEvent::Failed(io::ErrorKind::Interrupted)),
            Step {
                action: Action::Retry,
                note: false
            }
        );
    }

    #[test]
    fn repeated_errors_record_one_note() {
        let mut state = ReaderState::new(10);
        let notes = (0..5).filter(|_| state.next(failed()).note).count();
        assert_eq!(notes, 1);
    }

    #[test]
    fn the_first_error_notes_and_backs_off() {
        let mut state = ReaderState::new(3);
        assert_eq!(
            state.next(failed()),
            Step {
                action: Action::Backoff,
                note: true
            }
        );
    }

    #[test]
    fn errors_past_the_cap_hold_the_pipe() {
        let mut state = ReaderState::new(2);
        assert_eq!(state.next(failed()).action, Action::Backoff);
        assert_eq!(state.next(failed()).action, Action::Backoff);
        assert_eq!(state.next(failed()).action, Action::Hold);
    }

    #[test]
    fn after_an_error_the_action_is_never_end_until_eof() {
        let mut state = ReaderState::new(2);
        for _ in 0..6 {
            let action = state.next(failed()).action;
            assert_ne!(action, Action::End);
        }
        assert_ne!(
            state
                .next(ReadEvent::Failed(io::ErrorKind::Interrupted))
                .action,
            Action::End
        );
        assert_ne!(state.next(ReadEvent::Data).action, Action::End);
        assert_eq!(state.next(ReadEvent::Eof).action, Action::End);
    }

    #[test]
    fn data_resets_the_failure_count_but_not_the_note() {
        let mut state = ReaderState::new(1);
        assert!(state.next(failed()).note);
        assert_eq!(state.next(ReadEvent::Data).action, Action::Process);
        let step = state.next(failed());
        assert_eq!(step.action, Action::Backoff);
        assert!(!step.note);
    }

    #[test]
    fn an_interrupted_read_does_not_count_as_a_failure() {
        let mut state = ReaderState::new(1);
        for _ in 0..5 {
            state.next(ReadEvent::Failed(io::ErrorKind::Interrupted));
        }
        assert_eq!(state.next(failed()).action, Action::Backoff);
    }

    #[test]
    fn an_interrupted_read_retries() {
        assert_eq!(read_outcome(io::ErrorKind::Interrupted), ReadOutcome::Retry);
    }

    #[test]
    fn any_other_read_error_stops() {
        for kind in [
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::Other,
        ] {
            assert_eq!(read_outcome(kind), ReadOutcome::Stop, "{kind:?}");
        }
    }

    #[test]
    fn whole_lines_come_out_without_the_newline() {
        assert_eq!(split(64, &[b"one\ntwo\n"]), ["one", "two"]);
    }

    #[test]
    fn a_line_split_across_chunks_is_joined() {
        assert_eq!(
            split(64, &[b"ral", b"ph web\nne", b"xt\n"]),
            ["ralph web", "next"]
        );
    }

    #[test]
    fn invalid_utf8_is_replaced_not_fatal() {
        assert_eq!(
            split(64, &[b"\xff\xfe junk\nok\n"]),
            ["\u{fffd}\u{fffd} junk", "ok"]
        );
    }

    #[test]
    fn an_over_long_line_is_truncated_and_the_rest_discarded() {
        assert_eq!(split(4, &[b"abcdefgh", b"ijkl\nxy\n"]), ["abcd", "xy"]);
    }

    #[test]
    fn a_line_exactly_at_the_cap_is_kept_whole() {
        assert_eq!(split(4, &[b"abcd\nef\n"]), ["abcd", "ef"]);
    }

    #[test]
    fn a_last_line_without_a_newline_is_flushed_at_eof() {
        assert_eq!(split(64, &[b"one\ntail"]), ["one", "tail"]);
    }

    #[test]
    fn eof_after_a_newline_adds_nothing() {
        assert_eq!(split(64, &[b"one\n"]), ["one"]);
        assert!(split(64, &[]).is_empty());
    }

    #[test]
    fn a_crlf_ending_yields_the_line_without_the_cr() {
        assert_eq!(split(64, &[b"a\r\nb\r\n"]), ["a", "b"]);
        assert_eq!(split(64, &[b"a\r", b"\nb\n"]), ["a", "b"]);
    }

    #[test]
    fn a_lone_cr_inside_a_line_is_kept() {
        assert_eq!(split(64, &[b"a\rb\n"]), ["a\rb"]);
    }

    #[test]
    fn a_cr_at_eof_without_a_newline_is_kept() {
        assert_eq!(split(64, &[b"a\r"]), ["a\r"]);
    }

    #[test]
    fn a_bare_crlf_is_an_empty_line() {
        assert_eq!(split(64, &[b"\r\n"]), [""]);
    }

    #[test]
    fn empty_lines_are_kept() {
        assert_eq!(split(64, &[b"a\n\nb\n"]), ["a", "", "b"]);
    }

    #[test]
    fn a_truncated_line_without_a_newline_is_flushed_truncated() {
        assert_eq!(split(3, &[b"abcdef"]), ["abc"]);
    }

    #[test]
    fn truncation_can_cut_a_char_and_stays_lossy() {
        assert_eq!(split(1, &[b"\xc3\xa9\n"]), ["\u{fffd}"]);
    }
}
