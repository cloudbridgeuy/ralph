//! Warning message utilities.
//!
//! Centralized helpers for consistent warning output. On a terminal a warning
//! renders as an amber block with a left border bar and a warning symbol; when
//! stderr is not a terminal (or `NO_COLOR` is set) it is plain
//! "Warning: {message}" text.

use std::fmt::Display;
use std::io::IsTerminal;

const AMBER: &str = "\x1b[38;5;214m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const BAR: &str = "\u{258c}";
const SYMBOL: &str = "\u{26a0}";
/// Visible columns taken by the border and symbol prefix ("▌ ⚠ ").
const PREFIX_WIDTH: usize = 4;
const DEFAULT_TERMINAL_WIDTH: usize = 80;

/// Split `text` into lines of at most `width` characters (pure function).
///
/// Existing newlines are kept, words wrap at spaces, and words longer than
/// `width` are split.
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

/// Build the warning text (pure function).
///
/// With `color`, returns an amber bordered block wrapped to `terminal_width`;
/// otherwise returns `Warning: {message}`.
fn format_warning(message: &str, terminal_width: usize, color: bool) -> String {
    if !color {
        return format!("Warning: {message}");
    }
    let body_width = terminal_width.saturating_sub(PREFIX_WIDTH).max(1);
    wrap_text(message, body_width)
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let marker = if i == 0 {
                format!("{BOLD}{SYMBOL}{RESET}")
            } else {
                " ".to_string()
            };
            format!("{AMBER}{BAR}{RESET} {AMBER}{marker}{RESET} {line}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Print a warning message to stderr.
///
/// # Examples
///
/// ```no_run
/// use ralph::warn::warn;
///
/// warn("Not a git repository. Skipping diff capture.");
/// ```
pub fn warn(message: impl Display) {
    let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let width = crossterm::terminal::size()
        .map(|(w, _)| usize::from(w))
        .unwrap_or(DEFAULT_TERMINAL_WIDTH);
    eprintln!("{}", format_warning(&message.to_string(), width, color));
}

/// Log a warning if the result is an error, then discard the result.
///
/// Warns with "{context}: {error}".
///
/// # Arguments
///
/// * `result` - The Result to check
/// * `context` - A description of what operation failed
///
/// # Examples
///
/// ```no_run
/// use ralph::warn::warn_if_err;
/// use std::io;
///
/// fn may_fail() -> Result<(), io::Error> {
///     Err(io::Error::new(io::ErrorKind::NotFound, "file not found"))
/// }
///
/// warn_if_err(may_fail(), "Failed to finalize session");
/// // Warns: Failed to finalize session: file not found
/// ```
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
    fn colored_single_line_has_bar_symbol_and_amber() {
        let out = format_warning("disk full", 80, true);
        assert!(out.contains(AMBER));
        assert_eq!(strip_ansi(&out), "\u{258c} \u{26a0} disk full");
    }

    #[test]
    fn colored_wraps_under_border() {
        let out = format_warning("one two three four five six", 14, true);
        assert_eq!(
            strip_ansi(&out),
            "\u{258c} \u{26a0} one two\n\u{258c}   three four\n\u{258c}   five six"
        );
    }

    #[test]
    fn colored_keeps_newlines_and_splits_long_words() {
        let out = strip_ansi(&format_warning("ab\nabcdefghij", 9, true));
        assert_eq!(
            out,
            "\u{258c} \u{26a0} ab\n\u{258c}   abcde\n\u{258c}   fghij"
        );
    }

    #[test]
    fn test_warn_if_err_with_ok_does_not_panic() {
        let result: Result<i32, io::Error> = Ok(42);
        warn_if_err(result, "This should not print");
        // Test passes if no panic occurs
    }

    #[test]
    fn test_warn_if_err_with_err_does_not_panic() {
        let result: Result<i32, io::Error> =
            Err(io::Error::new(io::ErrorKind::NotFound, "test error"));
        warn_if_err(result, "Test context");
        // Test passes if no panic occurs
    }
}
