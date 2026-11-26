use unicode_width::UnicodeWidthStr;

use crate::ansi::{DIM, RESET};
use crate::chat_status::truncate_to_width;

const RULE: &str = "─";
const LEAD: &str = "── ";
const MARGIN: &str = "  ";
const KITTY_OPEN: &str = "\x1b_G";

#[derive(Debug, Clone, Copy)]
pub struct Columns(pub u16);

#[derive(Debug, Clone, Copy)]
pub enum Label<'a> {
    Bare,
    Language(&'a str),
}

pub fn rule_line(label: &Label<'_>, cols: Columns) -> String {
    let cols = usize::from(cols.0);
    let head = match label {
        Label::Bare => String::new(),
        Label::Language(name) => heading(name, cols),
    };
    let rest = cols.saturating_sub(head.width());
    head + &RULE.repeat(rest)
}

fn heading(name: &str, cols: usize) -> String {
    let room = cols.saturating_sub(LEAD.width() + 1);
    if room == 0 {
        String::new()
    } else if name.width() <= room {
        format!("{LEAD}{name} ")
    } else {
        format!("{LEAD}{}… ", truncate_to_width(name, room - 1))
    }
}

pub fn frame_block(label: &Label<'_>, body: &str, cols: Columns) -> String {
    let open = rule_line(label, cols);
    let close = rule_line(&Label::Bare, cols);
    let rows = body_rows(body);
    format!("{DIM}{open}{RESET}\n{rows}{DIM}{close}{RESET}\n")
}

fn body_rows(body: &str) -> String {
    body.split('\n')
        .map(|line| {
            if line.contains(KITTY_OPEN) {
                format!("{line}\n")
            } else {
                format!("{MARGIN}{line}\n")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    const LONG: &str = "averyveryverylonglanguagename";

    #[test]
    fn rule_line_carries_language() {
        let line = rule_line(&Label::Language("rust"), Columns(20));
        assert_eq!(line, format!("── rust {}", "─".repeat(12)));
    }

    #[test]
    fn rule_line_bare_is_solid() {
        assert_eq!(rule_line(&Label::Bare, Columns(5)), "─────");
    }

    #[test]
    fn rule_line_truncates_long_label() {
        for cols in 0..40 {
            let line = rule_line(&Label::Language(LONG), Columns(cols));
            assert_eq!(line.width(), usize::from(cols), "{line:?}");
        }
        let line = rule_line(&Label::Language(LONG), Columns(12));
        assert_eq!(line, "── averyve… ");
    }

    #[test]
    fn frame_indents_code_between_rules() {
        let out = frame_block(&Label::Language("sh"), "a\n\nb", Columns(8));
        assert_eq!(
            out,
            "\x1b[2m── sh ──\x1b[0m\n  a\n  \n  b\n\x1b[2m────────\x1b[0m\n"
        );
    }

    #[test]
    fn apc_row_gets_no_margin() {
        let apc = "\x1b_Ga=T;QQ==\x1b\\";
        let out = frame_block(&Label::Bare, &format!("{apc}\ntail"), Columns(4));
        assert!(out.contains(&format!("\n{apc}\n  tail\n")), "{out:?}");
    }
}
