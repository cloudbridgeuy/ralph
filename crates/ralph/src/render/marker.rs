pub(super) const LABEL_START: char = '\u{200B}';
pub(super) const LABEL_END: char = '\u{200C}';
pub(super) const URL_START: char = '\u{200D}';
pub(super) const URL_END: char = '\u{2060}';
pub(super) const MEMORY_START: char = '\u{2061}';
pub(super) const MEMORY_END: char = '\u{2062}';
pub(super) const ADD_START: char = '\u{2063}';
pub(super) const DEL_START: char = '\u{2064}';
pub(super) const DIFF_END: char = '\u{FEFF}';
pub(super) const DIFF_CODE_END: char = '\u{200E}';
pub(super) const PILL_END: char = '\u{E000}';
pub(super) const PILL_OK: char = '\u{E001}';
pub(super) const PILL_BAD: char = '\u{E002}';
pub(super) const PILL_WARN: char = '\u{E003}';
pub(super) const PILL_INFO: char = '\u{E004}';
pub(super) const PILL_ACCENT: char = '\u{E005}';
pub(super) const MARK_END: char = '\u{E006}';
pub(super) const MARK_DONE: char = '\u{E007}';
pub(super) const MARK_BAD: char = '\u{E008}';
pub(super) const MARK_ACCENT: char = '\u{E009}';
pub(super) const MARK_BORDER: char = '\u{E00A}';
pub(super) const MARKERS: [char; 21] = [
    LABEL_START,
    LABEL_END,
    URL_START,
    URL_END,
    MEMORY_START,
    MEMORY_END,
    ADD_START,
    DEL_START,
    DIFF_END,
    DIFF_CODE_END,
    PILL_END,
    PILL_OK,
    PILL_BAD,
    PILL_WARN,
    PILL_INFO,
    PILL_ACCENT,
    MARK_END,
    MARK_DONE,
    MARK_BAD,
    MARK_ACCENT,
    MARK_BORDER,
];

use crate::ansi::{bg, fg};
use crate::markdown::Palette;
use termimad::crossterm::style::Color;

const UNDERLINE: &str = "\x1b[4m";
const BOLD: &str = "\x1b[1m";
const PILL_OFF: &str = "\x1b[22m\x1b[39m\x1b[49m";
pub(super) const LABEL_STYLE_OFF: &str = "\x1b[24m";
pub(crate) const COLOR_OFF: &str = "\x1b[39m";

pub fn is_marker(ch: char) -> bool {
    MARKERS.contains(&ch)
}

pub fn paint_links(rendered: &str, palette: &Palette) -> String {
    let code_restore = match palette.inline_code {
        Some(_) => fg(palette.inline_code),
        None => COLOR_OFF.to_string(),
    };
    let starts = [
        (LABEL_START, format!("{UNDERLINE}{}", fg(palette.link))),
        (URL_START, fg(palette.url)),
        (MEMORY_START, fg(palette.memory_ref)),
        (ADD_START, fg(palette.done)),
        (DEL_START, fg(palette.blocked)),
        (PILL_OK, pill_style(palette.pill_ok)),
        (PILL_BAD, pill_style(palette.pill_bad)),
        (PILL_WARN, pill_style(palette.pill_warn)),
        (PILL_INFO, pill_style(palette.pill_info)),
        (PILL_ACCENT, pill_style(palette.pill_accent)),
        (MARK_DONE, fg(palette.done)),
        (MARK_BAD, fg(palette.blocked)),
        (MARK_ACCENT, fg(palette.pill_accent)),
        (MARK_BORDER, fg(palette.border)),
    ];
    let mut out = String::with_capacity(rendered.len());
    let mut open: Option<&str> = None;
    let mut in_label = false;
    for ch in rendered.chars() {
        if in_label && is_mark_marker(ch) {
            continue;
        }
        if let Some((_, style)) = starts.iter().find(|(start, _)| *start == ch) {
            in_label |= ch == LABEL_START;
            open = Some(style);
            out.push_str(style);
            continue;
        }
        match ch {
            LABEL_END => {
                in_label = false;
                open = None;
                out.push_str(LABEL_STYLE_OFF);
            }
            URL_END | MEMORY_END | DIFF_END | MARK_END => {
                open = None;
                out.push_str(COLOR_OFF);
            }
            DIFF_CODE_END => {
                open = None;
                out.push_str(&code_restore);
            }
            PILL_END => {
                open = None;
                out.push_str(PILL_OFF);
            }
            '\n' => {
                out.push('\n');
                out.push_str(open.unwrap_or_default());
            }
            _ => out.push(ch),
        }
    }
    out
}

fn is_mark_marker(ch: char) -> bool {
    matches!(
        ch,
        MARK_DONE | MARK_BAD | MARK_ACCENT | MARK_BORDER | MARK_END
    )
}

fn pill_style(tone: Option<Color>) -> String {
    format!("{BOLD}{}{}", bg(tone), fg(Some(contrast(tone))))
}

fn contrast(tone: Option<Color>) -> Color {
    let light = match tone {
        Some(Color::Rgb { r, g, b }) => {
            299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b) > 150_000
        }
        Some(Color::Green | Color::Yellow | Color::Cyan | Color::White | Color::Grey) => true,
        _ => false,
    };
    if light {
        Color::Black
    } else {
        Color::White
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_inside_a_link_label_keep_the_link_colour() {
        let palette = Palette::default();
        let marked = format!("{LABEL_START}{MARK_DONE}\u{2713}{MARK_END} done{LABEL_END}");
        let plain = format!("{LABEL_START}\u{2713} done{LABEL_END}");
        assert_eq!(
            paint_links(&marked, &palette),
            paint_links(&plain, &palette)
        );
    }

    #[test]
    fn marks_after_a_link_label_are_painted_again() {
        let palette = Palette::default();
        let marked = format!("{LABEL_START}a{LABEL_END} {MARK_DONE}\u{2713}{MARK_END}");
        let painted = paint_links(&marked, &palette);
        assert!(painted.ends_with(&format!("{}\u{2713}{COLOR_OFF}", fg(palette.done))));
    }
}
