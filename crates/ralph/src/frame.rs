use crate::ansi::{fg, BOLD, DIM, RESET};
use crate::markdown::Palette;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const FRAME_WIDTH: usize = 4;
const ELLIPSIS: char = '…';
const STATUS_DOT: &str = "●";

pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_escape = false;
    for ch in text.chars() {
        match (in_escape, ch) {
            (false, '\x1b') => in_escape = true,
            (true, ch) if ch.is_ascii_alphabetic() => in_escape = false,
            (false, ch) => out.push(ch),
            _ => {}
        }
    }
    out
}

pub fn display_width(text: &str) -> usize {
    strip_ansi(text).width()
}

pub fn frame(rows: &[String], border: &str) -> String {
    let inner = rows.iter().map(|row| display_width(row)).max().unwrap_or(0);
    let rule = "─".repeat(inner + 2);
    let body = rows.iter().map(|row| {
        let pad = " ".repeat(inner - display_width(row));
        format!("{border}│{RESET} {row}{pad} {border}│{RESET}")
    });
    std::iter::once(format!("{border}╭{rule}╮{RESET}"))
        .chain(body)
        .chain(std::iter::once(format!("{border}╰{rule}╯{RESET}")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_to_width(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = ch.width().unwrap_or(0);
        if used + width + 1 > max {
            break;
        }
        out.push(ch);
        used += width;
    }
    if max > 0 {
        out.push(ELLIPSIS);
    }
    out
}

fn status_color(status: &str, palette: &Palette) -> String {
    fg(match status {
        "done" => palette.done,
        "blocked" | "failed" => palette.blocked,
        "waiting" | "check" => palette.waiting,
        _ => None,
    })
}

pub fn worker_header(
    worker: &str,
    status: &str,
    terminal_width: usize,
    palette: Option<&Palette>,
) -> String {
    let Some(palette) = palette else {
        return format!("[worker {worker} · {status}]");
    };
    let fixed = format!("{STATUS_DOT} worker  · {status}").width();
    let room = terminal_width.saturating_sub(FRAME_WIDTH + fixed);
    let worker = truncate_to_width(worker, room);
    let tint = status_color(status, palette);
    let row = format!(
        "{tint}{STATUS_DOT}{RESET} {DIM}worker{RESET} {BOLD}{worker}{RESET} {DIM}·{RESET} {BOLD}{tint}{status}{RESET}"
    );
    frame(&[row], &fg(palette.border))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::{BLUE, GREEN, RED, YELLOW};
    use termimad::crossterm::style::Color;

    fn colored(worker: &str, status: &str, width: usize) -> String {
        worker_header(worker, status, width, Some(&Palette::default()))
    }

    #[test]
    fn plain_header_when_not_color() {
        assert_eq!(
            worker_header("GUZ-476", "done", 80, None),
            "[worker GUZ-476 · done]"
        );
    }

    #[test]
    fn colored_header_is_boxed() {
        assert_eq!(
            strip_ansi(&colored("GUZ-476", "done", 80)),
            "╭─────────────────────────╮\n│ ● worker GUZ-476 · done │\n╰─────────────────────────╯"
        );
    }

    #[test]
    fn status_colors_follow_meaning() {
        let palette = Palette::default();
        assert_eq!(status_color("done", &palette), GREEN);
        assert_eq!(status_color("blocked", &palette), RED);
        assert_eq!(status_color("failed", &palette), RED);
        assert_eq!(status_color("waiting", &palette), YELLOW);
        assert_eq!(status_color("check", &palette), YELLOW);
        assert_eq!(status_color("question", &palette), "");
        assert!(colored("a", "blocked", 80).contains(RED));
        assert!(colored("a", "done", 80).starts_with(BLUE));
    }

    #[test]
    fn header_uses_palette_border_and_status() {
        let palette = Palette {
            border: Some(Color::Magenta),
            done: Some(Color::Cyan),
            ..Palette::default()
        };
        let out = worker_header("a", "done", 80, Some(&palette));
        assert!(out.starts_with("\x1b[95m"));
        assert!(out.contains("\x1b[96m"));
        assert!(!out.contains(GREEN));
    }

    #[test]
    fn wide_worker_ids_align_by_display_width() {
        let out = strip_ansi(&colored("作業", "done", 80));
        let widths: Vec<usize> = out.lines().map(UnicodeWidthStr::width).collect();
        assert!(widths.iter().all(|w| *w == widths[0]));
    }

    #[test]
    fn long_worker_ids_are_cut_to_fit_terminal() {
        let out = strip_ansi(&colored(&"x".repeat(100), "done", 30));
        assert!(out.lines().all(|line| line.width() <= 30));
        assert!(out.contains('…'));
    }

    #[test]
    fn truncate_to_width_keeps_room_for_the_ellipsis() {
        assert_eq!(truncate_to_width("abc", 3), "abc");
        assert_eq!(truncate_to_width("abcdef", 4), "abc…");
        assert_eq!(truncate_to_width("作業x", 3), "作…");
    }

    #[test]
    fn truncate_to_width_zero_is_empty() {
        assert_eq!(truncate_to_width("abc", 0), "");
    }

    #[test]
    fn frame_pads_rows_to_widest() {
        assert_eq!(
            strip_ansi(&frame(&["ab".to_string(), "a".to_string()], "")),
            "╭────╮\n│ ab │\n│ a  │\n╰────╯"
        );
    }
}
