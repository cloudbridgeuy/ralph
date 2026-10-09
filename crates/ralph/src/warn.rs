use crate::ansi::{BOLD, RESET, YELLOW};
use std::fmt::Display;
use std::io::IsTerminal;

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
    let lines = wrap_text(message, body_width);
    let text_width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let rule = "\u{2500}".repeat(text_width + 4);
    let body = lines.iter().enumerate().map(|(i, line)| {
        let marker = if i == 0 {
            format!("{BOLD}{YELLOW}{SYMBOL}{RESET}")
        } else {
            " ".to_string()
        };
        let pad = " ".repeat(text_width - line.chars().count());
        format!("{YELLOW}\u{2502}{RESET} {marker} {line}{pad} {YELLOW}\u{2502}{RESET}")
    });
    std::iter::once(format!("{YELLOW}\u{256d}{rule}\u{256e}{RESET}"))
        .chain(body)
        .chain(std::iter::once(format!(
            "{YELLOW}\u{2570}{rule}\u{256f}{RESET}"
        )))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn warn(message: impl Display) {
    let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let width = crossterm::terminal::size()
        .map(|(w, _)| usize::from(w))
        .unwrap_or(DEFAULT_TERMINAL_WIDTH);
    eprintln!("{}", format_warning(&message.to_string(), width, color));
}

pub fn warn_if_err<T, E: Display>(result: Result<T, E>, context: impl Display) {
    if let Err(e) = result {
        warn(format!("{context}: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut in_escape = false;
        for c in s.chars() {
            match (in_escape, c) {
                (false, '\x1b') => in_escape = true,
                (true, 'm') => in_escape = false,
                (false, c) => out.push(c),
                _ => {}
            }
        }
        out
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
