use std::borrow::Cow;

/// Drop NUL and other Unicode controls. Newline and tab stay.
pub fn strip_controls(text: &str) -> Cow<'_, str> {
    if text.chars().any(forbidden_control) {
        Cow::Owned(text.chars().filter(|c| !forbidden_control(*c)).collect())
    } else {
        Cow::Borrowed(text)
    }
}

fn forbidden_control(c: char) -> bool {
    c.is_control() && c != '\n' && c != '\t'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_nul_and_other_controls_and_keeps_newline_tab_and_backtick() {
        let cleaned = strip_controls("a\0b\u{0001}c\r\n\td`e\u{007f}\u{0080}");
        assert_eq!(cleaned, "abc\n\td`e");
    }

    #[test]
    fn leaves_clean_text_unchanged() {
        let text = "Install: `curl`\n\tok";
        assert_eq!(strip_controls(text), text);
    }

    #[test]
    fn a_split_backtick_escape_loses_only_the_nul() {
        assert_eq!(strip_controls("\u{0000}60curl"), "60curl");
    }
}
