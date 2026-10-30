use crate::ansi::{BOLD, CLEAR_EOL, CR, RESET, YELLOW};
use crate::frame::frame;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::io::{self, IsTerminal, Write};
use std::sync::mpsc::SyncSender;
use std::sync::Mutex;

const SYMBOL: &str = "\u{26a0}";
const FRAME_WIDTH: usize = 6;
const DEFAULT_TERMINAL_WIDTH: usize = 80;

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut current = String::new();
        let mut current_len = 0;
        for word in paragraph.split_whitespace() {
            let mut word: Vec<char> = word.chars().collect();
            while word.len() > width {
                if current_len > 0 {
                    lines.push(std::mem::take(&mut current));
                    current_len = 0;
                }
                lines.push(word.drain(..width).collect());
            }
            let word_len = word.len();
            if word_len == 0 {
                continue;
            }
            if current_len > 0 && current_len + 1 + word_len > width {
                lines.push(std::mem::take(&mut current));
                current_len = 0;
            }
            if current_len > 0 {
                current.push(' ');
                current_len += 1;
            }
            current.extend(word);
            current_len += word_len;
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn format_warning(message: &str, terminal_width: usize, color: bool) -> String {
    if !color {
        return format!("Warning: {message}");
    }
    let body_width = terminal_width.saturating_sub(FRAME_WIDTH).max(1);
    let rows: Vec<String> = wrap_text(message, body_width)
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let marker = if i == 0 {
                format!("{BOLD}{YELLOW}{SYMBOL}{RESET}")
            } else {
                " ".to_string()
            };
            format!("{marker} {line}")
        })
        .collect();
    frame(&rows, YELLOW)
}

fn warning_text(message: impl Display) -> (String, bool) {
    let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let width = crossterm::terminal::size()
        .map(|(w, _)| usize::from(w))
        .unwrap_or(DEFAULT_TERMINAL_WIDTH);
    (format_warning(&message.to_string(), width, color), color)
}

pub fn warning_line(message: impl Display) -> String {
    let (text, color) = warning_text(message);
    line_of(&text, color)
}

fn line_of(text: &str, clear: bool) -> String {
    let prefix = if clear {
        format!("{CR}{CLEAR_EOL}")
    } else {
        String::new()
    };
    format!("{prefix}{text}\n")
}

static PROMPT: Mutex<Option<SyncSender<String>>> = Mutex::new(None);

pub fn print_above_prompt(sink: Option<SyncSender<String>>) {
    if let Ok(mut prompt) = PROMPT.lock() {
        *prompt = sink;
    }
}

fn prompt_rows(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

fn deliver(
    text: &str,
    clear: bool,
    sink: Option<&SyncSender<String>>,
    out: &mut impl Write,
) -> io::Result<()> {
    let rows = prompt_rows(text);
    let sent = sink.map_or(0, |sink| {
        rows.iter()
            .take_while(|row| sink.try_send(row.to_string()).is_ok())
            .count()
    });
    if sent == rows.len() {
        return Ok(());
    }
    out.write_all(line_of(&rows[sent..].join("\n"), clear).as_bytes())
}

pub fn warn(message: impl Display) {
    let (text, color) = warning_text(message);
    let sink = PROMPT.lock().ok().and_then(|prompt| prompt.clone());
    deliver(&text, color, sink.as_ref(), &mut io::stderr()).ok();
}

static SEEN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

fn first_sighting(seen: &mut BTreeSet<String>, key: &str) -> bool {
    seen.insert(key.to_string())
}

pub fn warn_once(key: &str, message: impl Display) {
    let fresh = match SEEN.lock() {
        Ok(mut seen) => first_sighting(&mut seen, key),
        Err(_) => true,
    };
    if fresh {
        warn(message);
    }
}

pub fn warn_if_err<T, E: Display>(result: Result<T, E>, context: impl Display) {
    if let Err(e) = result {
        warn(format!("{context}: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::strip_ansi;
    use reedline::ExternalPrinter;
    use std::io;

    const BOX: &str = "\u{256d}\u{2500}\u{256e}\n\u{2502}x\u{2502}\n\u{2570}\u{2500}\u{256f}";

    #[test]
    fn line_of_ends_with_one_newline() {
        assert_eq!(line_of("Warning: x", false), "Warning: x\n");
    }

    #[test]
    fn line_of_clears_the_line_first_when_asked() {
        assert_eq!(line_of("x", true), "\r\x1b[Kx\n");
    }

    #[test]
    fn prompt_rows_split_the_box_into_bare_rows() {
        let rows = prompt_rows(&format_warning("codex session GUZ-881 resumed", 80, true));
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| !row.contains(['\r', '\n'])));
    }

    #[test]
    fn deliver_sends_every_row_above_the_prompt() {
        let printer = ExternalPrinter::<String>::new(8);
        let mut out = Vec::new();
        deliver(BOX, true, Some(&printer.sender()), &mut out).unwrap();
        let sent: Vec<String> = printer.receiver().try_iter().collect();
        assert_eq!(sent, prompt_rows(BOX));
        assert!(out.is_empty());
    }

    #[test]
    fn deliver_without_a_prompt_writes_the_line() {
        let mut out = Vec::new();
        deliver(BOX, true, None, &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), line_of(BOX, true));
    }

    #[test]
    fn deliver_writes_the_rows_a_full_prompt_drops() {
        let printer = ExternalPrinter::<String>::new(1);
        let mut out = Vec::new();
        deliver(BOX, false, Some(&printer.sender()), &mut out).unwrap();
        let rows = prompt_rows(BOX);
        assert_eq!(printer.receiver().try_iter().collect::<Vec<_>>(), rows[..1]);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            line_of(&rows[1..].join("\n"), false)
        );
    }

    #[test]
    fn plain_format_when_not_color() {
        assert_eq!(format_warning("disk full", 80, false), "Warning: disk full");
    }

    #[test]
    fn colored_single_line_is_boxed() {
        let out = format_warning("disk full", 80, true);
        assert!(out.contains(YELLOW));
        assert_eq!(
            strip_ansi(&out),
            "\u{256d}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256e}\n\u{2502} \u{26a0} disk full \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256f}"
        );
    }

    #[test]
    fn colored_wrap_keeps_borders_aligned_within_width() {
        let out = strip_ansi(&format_warning("one two three four five six", 16, true));
        let widths: Vec<usize> = out.lines().map(|l| l.chars().count()).collect();
        assert!(widths.len() > 3);
        assert!(widths.iter().all(|w| *w == widths[0] && *w <= 16));
    }

    #[test]
    fn colored_keeps_newlines_and_splits_long_words() {
        let out = strip_ansi(&format_warning("ab\nabcdefghij", 11, true));
        let rows: Vec<&str> = out.lines().skip(1).take(3).collect();
        assert_eq!(
            rows,
            [
                "\u{2502} \u{26a0} ab    \u{2502}",
                "\u{2502}   abcde \u{2502}",
                "\u{2502}   fghij \u{2502}"
            ]
        );
    }

    #[test]
    fn first_sighting_is_true_once_per_key() {
        let mut seen = BTreeSet::new();
        assert!(first_sighting(&mut seen, "a"));
        assert!(!first_sighting(&mut seen, "a"));
        assert!(first_sighting(&mut seen, "b"));
    }

    #[test]
    fn test_warn_if_err_with_ok_does_not_panic() {
        let result: Result<i32, io::Error> = Ok(42);
        warn_if_err(result, "This should not print");
    }

    #[test]
    fn test_warn_if_err_with_err_does_not_panic() {
        let result: Result<i32, io::Error> =
            Err(io::Error::new(io::ErrorKind::NotFound, "test error"));
        warn_if_err(result, "Test context");
    }
}
