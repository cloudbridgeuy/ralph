use crate::chat_cell::OUTPUT_START;

const ESC: char = '\u{1b}';
const SGR_PARAMS_MAX: usize = 64;
const SGR_RESET: &str = "\u{1b}[0m";

pub struct PreambleGate {
    open: bool,
}

impl PreambleGate {
    pub fn new(stateful: bool) -> Self {
        Self { open: !stateful }
    }

    pub fn feed<'a>(&mut self, chunk: &'a [u8]) -> &'a [u8] {
        if self.open {
            return chunk;
        }
        match chunk
            .iter()
            .position(|&byte| char::from(byte) == OUTPUT_START)
        {
            Some(at) => {
                self.open = true;
                &chunk[at + 1..]
            }
            None => &[],
        }
    }
}

#[derive(Default)]
enum Escape {
    #[default]
    None,
    Start,
    Intermediate,
    Csi(String),
    CsiDropped,
    String,
    StringEsc,
}

pub struct Sanitizer {
    keep_sgr: bool,
    pending: Vec<u8>,
    escape: Escape,
    after_cr: bool,
    styled: bool,
}

impl Sanitizer {
    pub fn new(keep_sgr: bool) -> Self {
        Self {
            keep_sgr,
            pending: Vec::new(),
            escape: Escape::None,
            after_cr: false,
            styled: false,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> String {
        let mut input = std::mem::take(&mut self.pending);
        input.extend_from_slice(bytes);
        let mut out = String::new();
        let mut rest = input.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    text.chars().for_each(|c| self.step(c, &mut out));
                    break;
                }
                Err(error) => {
                    let (valid, invalid) = rest.split_at(error.valid_up_to());
                    String::from_utf8_lossy(valid)
                        .chars()
                        .for_each(|c| self.step(c, &mut out));
                    match error.error_len() {
                        Some(len) => {
                            self.step(char::REPLACEMENT_CHARACTER, &mut out);
                            rest = &invalid[len..];
                        }
                        None => {
                            self.pending = invalid.to_vec();
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        if !std::mem::take(&mut self.pending).is_empty() {
            self.step(char::REPLACEMENT_CHARACTER, &mut out);
        }
        self.escape = Escape::None;
        self.after_cr = false;
        if std::mem::take(&mut self.styled) {
            out.push_str(SGR_RESET);
        }
        out
    }

    fn step(&mut self, c: char, out: &mut String) {
        let after_cr = std::mem::take(&mut self.after_cr);
        match std::mem::take(&mut self.escape) {
            Escape::None => self.text(c, after_cr, out),
            Escape::Start => match c {
                '[' => self.escape = Escape::Csi(String::new()),
                ']' | 'P' | 'X' | '^' | '_' => self.escape = Escape::String,
                ' '..='/' => self.escape = Escape::Intermediate,
                '0'..='~' => {}
                _ => self.text(c, false, out),
            },
            Escape::Intermediate => match c {
                ' '..='/' => self.escape = Escape::Intermediate,
                '0'..='~' => {}
                _ => self.text(c, false, out),
            },
            Escape::Csi(mut params) => match c {
                ' '..='?' if params.len() < SGR_PARAMS_MAX => {
                    params.push(c);
                    self.escape = Escape::Csi(params);
                }
                ' '..='?' => self.escape = Escape::CsiDropped,
                'm' => self.sgr(&params, out),
                '@'..='~' => {}
                _ => self.text(c, false, out),
            },
            Escape::CsiDropped => match c {
                ' '..='?' => self.escape = Escape::CsiDropped,
                '@'..='~' => {}
                _ => self.text(c, false, out),
            },
            Escape::String => match c {
                '\u{7}' | '\u{9c}' => {}
                ESC => self.escape = Escape::StringEsc,
                _ => self.escape = Escape::String,
            },
            Escape::StringEsc => match c {
                '\\' => {}
                _ => {
                    self.escape = Escape::Start;
                    self.step(c, out);
                }
            },
        }
    }

    fn text(&mut self, c: char, after_cr: bool, out: &mut String) {
        match c {
            ESC => self.escape = Escape::Start,
            '\n' if after_cr => {}
            '\r' => {
                out.push('\n');
                self.after_cr = true;
            }
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }

    fn sgr(&mut self, params: &str, out: &mut String) {
        let plain = params
            .chars()
            .all(|c| c.is_ascii_digit() || c == ';' || c == ':');
        if self.keep_sgr && plain {
            out.push(ESC);
            out.push('[');
            out.push_str(params);
            out.push('m');
            self.styled = true;
        }
    }
}

pub fn ends_at_line_start(clean: &str, before: bool) -> bool {
    let mut at_start = before;
    let mut in_sgr = false;
    for c in clean.chars() {
        match (in_sgr, c) {
            (false, ESC) => in_sgr = true,
            (true, 'm') => in_sgr = false,
            (true, _) => {}
            (false, c) => at_start = c == '\n',
        }
    }
    at_start
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(input: &[u8]) -> String {
        let mut sanitizer = Sanitizer::new(true);
        sanitizer.feed(input) + &sanitizer.finish()
    }

    fn clean_split_at(input: &[u8], at: usize) -> String {
        let mut sanitizer = Sanitizer::new(true);
        let (head, tail) = input.split_at(at);
        sanitizer.feed(head) + &sanitizer.feed(tail) + &sanitizer.finish()
    }

    #[test]
    fn printable_text_newline_and_tab_pass() {
        assert_eq!(clean("a\tb\nçé ✓\n".as_bytes()), "a\tb\nçé ✓\n");
    }

    #[test]
    fn sgr_colour_is_kept_and_reset_at_finish() {
        assert_eq!(
            clean(b"\x1b[1;31mred\x1b[0m ok\x1b[38:5:2m"),
            "\x1b[1;31mred\x1b[0m ok\x1b[38:5:2m\x1b[0m"
        );
        assert_eq!(clean(b"plain"), "plain");
    }

    #[test]
    fn sgr_is_dropped_when_colour_is_off() {
        let mut sanitizer = Sanitizer::new(false);
        assert_eq!(sanitizer.feed(b"\x1b[31mred\x1b[0m"), "red");
        assert_eq!(sanitizer.finish(), "");
    }

    #[test]
    fn csi_other_than_plain_sgr_is_dropped() {
        assert_eq!(
            clean(b"\x1b[2J\x1b[H\x1b[5Ax\x1b[?25l\x1b[>4;1m\x1b[1 qy"),
            "xy"
        );
    }

    #[test]
    fn osc_title_clipboard_and_hyperlinks_are_dropped() {
        assert_eq!(
            clean(b"\x1b]0;evil\x07a\x1b]52;c;ZXZpbA==\x1b\\b\x1b]8;;http://x\x1b\\c"),
            "abc"
        );
    }

    #[test]
    fn dcs_and_kitty_graphics_are_dropped() {
        assert_eq!(
            clean(b"\x1bPq#0;2\x1b\\a\x1b_Gf=100,a=T;QUFB\x1b\\b\x1b^pm\x1b\\c\x1bXsos\x1b\\d"),
            "abcd"
        );
    }

    #[test]
    fn bare_and_two_char_escapes_are_dropped() {
        assert_eq!(
            clean(b"\x1bcA\x1b(BB\x1b7C\x1b\x1b[31mD"),
            "ABC\x1b[31mD\x1b[0m"
        );
        assert_eq!(clean(b"E\x1b"), "E");
    }

    #[test]
    fn other_controls_are_dropped() {
        assert_eq!(clean("a\x07\x08\x7f\x00b\u{9b}2Jc".as_bytes()), "ab2Jc");
    }

    #[test]
    fn carriage_returns_become_newlines_and_crlf_is_one() {
        assert_eq!(clean(b"a\r\nb\rc\r\r\n"), "a\nb\nc\n\n");
        assert_eq!(clean_split_at(b"a\r\nb", 2), "a\nb");
    }

    #[test]
    fn invalid_utf8_becomes_replacement_chars() {
        assert_eq!(clean(b"a\xffb\xc3"), "a\u{fffd}b\u{fffd}");
    }

    #[test]
    fn overlong_csi_is_dropped_without_leaking() {
        let mut input = b"\x1b[".to_vec();
        input.extend("1;".repeat(200).as_bytes());
        input.extend(b"mX");
        assert_eq!(clean(&input), "X");
    }

    #[test]
    fn every_chunk_split_gives_the_same_text() {
        let input =
            "\x1b[1;31mré\x1b[0m\r\n\x1b]0;t\x07\x1b]52;c;QQ==\x1b\\\x1bPq\x1b\\\x1b[2Jé✓\x1b(B.\n";
        let whole = clean(input.as_bytes());
        assert_eq!(whole, "\x1b[1;31mré\x1b[0m\né✓.\n\x1b[0m");
        for at in 0..=input.len() {
            assert_eq!(clean_split_at(input.as_bytes(), at), whole, "split at {at}");
        }
    }

    #[test]
    fn gate_holds_output_until_the_marker() {
        let mut gate = PreambleGate::new(true);
        assert_eq!(gate.feed(b"rc noise\n"), b"");
        assert_eq!(gate.feed(b"more\x1ehi"), b"hi");
        assert_eq!(gate.feed(b"\x1eagain"), b"\x1eagain");
    }

    #[test]
    fn stateless_gate_is_open() {
        let mut gate = PreambleGate::new(false);
        assert_eq!(gate.feed(b"rc\x1ehi"), b"rc\x1ehi");
    }

    #[test]
    fn line_start_ignores_colour_sequences() {
        assert!(ends_at_line_start("a\n\x1b[0m", false));
        assert!(!ends_at_line_start("a\x1b[0m", true));
        assert!(ends_at_line_start("\x1b[0m", true));
        assert!(!ends_at_line_start("", false));
    }
}
