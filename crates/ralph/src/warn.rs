use crate::ansi::{BOLD, CLEAR_EOL, CR, RESET, YELLOW};
use crate::frame::frame;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::io::IsTerminal;
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

pub fn warning_line(message: impl Display) -> String {
    let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let width = crossterm::terminal::size()
        .map(|(w, _)| usize::from(w))
        .unwrap_or(DEFAULT_TERMINAL_WIDTH);
    line_of(&format_warning(&message.to_string(), width, color), color)
}

fn line_of(text: &str, clear: bool) -> String {
    let prefix = if clear {
        format!("{CR}{CLEAR_EOL}")
    } else {
        String::new()
    };
    format!("{prefix}{text}\n")
}

pub fn warn(message: impl Display) {
    eprint!("{}", warning_line(message));
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
    use std::io;

    #[test]
    fn line_of_ends_with_one_newline() {
        assert_eq!(line_of("Warning: x", false), "Warning: x\n");
    }

    #[test]
    fn line_of_clears_the_line_first_when_asked() {
        assert_eq!(line_of("x", true), "\r\x1b[Kx\n");
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
