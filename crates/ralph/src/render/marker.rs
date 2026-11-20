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
pub(super) const MARKERS: [char; 10] = [
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
];

use crate::ansi::fg;
use crate::markdown::Palette;

const UNDERLINE: &str = "\x1b[4m";
pub(super) const LABEL_STYLE_OFF: &str = "\x1b[24m";
pub(crate) const COLOR_OFF: &str = "\x1b[39m";

pub fn is_marker(ch: char) -> bool {
    MARKERS.contains(&ch)
}

pub fn paint_links(rendered: &str, palette: &Palette) -> String {
    let label = format!("{UNDERLINE}{}", fg(palette.link));
    let url = fg(palette.url);
    let memory = fg(palette.memory_ref);
    let add = fg(palette.done);
    let del = fg(palette.blocked);
    let code_restore = match palette.inline_code {
        Some(_) => fg(palette.inline_code),
        None => COLOR_OFF.to_string(),
    };
    let mut out = String::with_capacity(rendered.len());
    let mut open: Option<&str> = None;
    for ch in rendered.chars() {
        match ch {
            LABEL_START => open = Some(&label),
            URL_START => open = Some(&url),
            MEMORY_START => open = Some(&memory),
            ADD_START => open = Some(&add),
            DEL_START => open = Some(&del),
            LABEL_END | URL_END | MEMORY_END | DIFF_END | DIFF_CODE_END => open = None,
            _ => {}
        }
        match ch {
            LABEL_START | URL_START | MEMORY_START | ADD_START | DEL_START => {
                out.push_str(open.unwrap_or_default())
            }
            LABEL_END => out.push_str(LABEL_STYLE_OFF),
            URL_END | MEMORY_END | DIFF_END => out.push_str(COLOR_OFF),
            DIFF_CODE_END => out.push_str(&code_restore),
            '\n' => {
                out.push('\n');
                out.push_str(open.unwrap_or_default());
            }
            _ => out.push(ch),
        }
    }
    out
}
