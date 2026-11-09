use crate::ansi::{fg, BOLD, CLEAR_EOL, CR, RESET};
use crate::frame::frame;
use crate::ui_theme::{DEFAULT_NOTICE_COLOR, DEFAULT_WARNING_COLOR};
use ralph_server::warn::WARNING_PREFIX;
pub use ralph_server::warn::{warn, warn_if_err, warn_once};
use std::fmt::Display;
use std::io::{self, IsTerminal, Write};
use std::sync::mpsc::SyncSender;
use std::sync::{Mutex, OnceLock};
use termimad::crossterm::style::Color;

const SYMBOL: &str = "\u{26a0}";
const CHECK: &str = "\u{2713}";
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

struct Style {
    symbol: &'static str,
    plain: &'static str,
    color: &'static OnceLock<Color>,
    default: Color,
}

static WARNING_COLOR: OnceLock<Color> = OnceLock::new();
static NOTICE_COLOR: OnceLock<Color> = OnceLock::new();

const WARNING: Style = Style {
    symbol: SYMBOL,
    plain: WARNING_PREFIX,
    color: &WARNING_COLOR,
    default: DEFAULT_WARNING_COLOR,
};

const NOTICE: Style = Style {
    symbol: CHECK,
    plain: CHECK,
    color: &NOTICE_COLOR,
    default: DEFAULT_NOTICE_COLOR,
};

pub fn set_warning_color(color: Color) {
    let _ = WARNING_COLOR.set(color);
}

pub fn set_notice_color(color: Color) {
    let _ = NOTICE_COLOR.set(color);
}

fn format_box(style: &Style, message: &str, terminal_width: usize, color: Option<Color>) -> String {
    let Some(color) = color else {
        return format!("{} {message}", style.plain);
    };
    let accent = fg(Some(color));
    let body_width = terminal_width.saturating_sub(FRAME_WIDTH).max(1);
    let rows: Vec<String> = wrap_text(message, body_width)
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let marker = if i == 0 {
                format!("{BOLD}{accent}{}{RESET}", style.symbol)
            } else {
                " ".to_string()
            };
            format!("{marker} {line}")
        })
        .collect();
    frame(&rows, &accent)
}

fn boxed_text(style: &Style, message: impl Display) -> (String, bool) {
    let color = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let width = crossterm::terminal::size()
        .map(|(w, _)| usize::from(w))
        .unwrap_or(DEFAULT_TERMINAL_WIDTH);
    let accent = color.then(|| style.color.get().copied().unwrap_or(style.default));
    (
        format_box(style, &message.to_string(), width, accent),
        color,
    )
}

pub fn warning_line(message: impl Display) -> String {
    let (text, color) = boxed_text(&WARNING, message);
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

fn print_boxed(style: &Style, message: impl Display) {
    let (text, color) = boxed_text(style, message);
    let sink = PROMPT.lock().ok().and_then(|prompt| prompt.clone());
    deliver(&text, color, sink.as_ref(), &mut io::stderr()).ok();
}

pub fn print_warning(message: &str) {
    print_boxed(&WARNING, message);
}

pub fn notice(message: impl Display) {
    print_boxed(&NOTICE, message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::{GREEN, YELLOW};
    use crate::frame::strip_ansi;
    use crate::ui_theme::UiColors;
    use reedline::ExternalPrinter;
    use termimad::crossterm::style::Color;

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
        let rows = prompt_rows(&format_box(
            &WARNING,
            "codex session GUZ-881 resumed",
            80,
            Some(Color::DarkYellow),
        ));
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
        assert_eq!(
            format_box(&WARNING, "disk full", 80, None),
            "Warning: disk full"
        );
    }

    #[test]
    fn colored_single_line_is_boxed() {
        let out = format_box(&WARNING, "disk full", 80, Some(Color::DarkYellow));
        assert!(out.contains(YELLOW));
        assert_eq!(
            strip_ansi(&out),
            "\u{256d}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256e}\n\u{2502} \u{26a0} disk full \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256f}"
        );
    }

    #[test]
    fn colored_wrap_keeps_borders_aligned_within_width() {
        let out = strip_ansi(&format_box(
            &WARNING,
            "one two three four five six",
            16,
            Some(Color::DarkYellow),
        ));
        let widths: Vec<usize> = out.lines().map(|l| l.chars().count()).collect();
        assert!(widths.len() > 3);
        assert!(widths.iter().all(|w| *w == widths[0] && *w <= 16));
    }

    #[test]
    fn colored_keeps_newlines_and_splits_long_words() {
        let out = strip_ansi(&format_box(
            &WARNING,
            "ab\nabcdefghij",
            11,
            Some(Color::DarkYellow),
        ));
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
    fn warning_color_reaches_the_symbol_and_the_border() {
        let out = format_box(&WARNING, "x", 80, Some(Color::Rgb { r: 1, g: 2, b: 3 }));
        assert!(out.starts_with("\x1b[38;2;1;2;3m\u{256d}"));
        assert!(out.contains("\x1b[1m\x1b[38;2;1;2;3m\u{26a0}"));
        assert!(!out.contains(YELLOW));
    }

    #[test]
    fn default_warning_color_keeps_todays_bytes() {
        let out = format_box(&WARNING, "x", 80, Some(DEFAULT_WARNING_COLOR));
        assert!(out.contains(&format!("{BOLD}{YELLOW}{SYMBOL}{RESET}")));
        assert!(out.starts_with(&format!("{YELLOW}\u{256d}")));
    }

    #[test]
    fn no_color_means_no_escape_byte() {
        assert!(!format_box(&WARNING, "x", 80, None).contains('\x1b'));
    }

    #[test]
    fn notice_box_has_a_green_check_and_border() {
        let out = format_box(
            &NOTICE,
            "codex session GUZ-881 resumed",
            80,
            Some(Color::DarkGreen),
        );
        assert!(out.starts_with(&format!("{GREEN}\u{256d}")));
        assert!(out.contains(&format!("{BOLD}{GREEN}\u{2713}{RESET}")));
        assert!(!out.contains(YELLOW));
        assert!(!out.contains(SYMBOL));
    }

    #[test]
    fn plain_notice_keeps_the_check_without_color() {
        assert_eq!(format_box(&NOTICE, "x", 80, None), "\u{2713} x");
    }

    #[test]
    fn notice_color_defaults_to_ask_ok() {
        assert_eq!(NOTICE.default, UiColors::default().ask_ok);
    }
}
