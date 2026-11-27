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
