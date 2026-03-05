//! Pure formatting functions for display values.

// Re-export shared formatting functions for backward compatibility
pub use crate::formatting::{format_duration, format_token_count};

/// Minimum separator width to avoid visually narrow banners.
const MIN_SEPARATOR_WIDTH: usize = 40;

/// Calculate the separator line width based on text content and terminal width.
///
/// - If text fits on one line: `max(text_char_count, MIN_SEPARATOR_WIDTH)`
/// - If text wraps (exceeds terminal width): `terminal_width`
///
/// Pure function — no I/O.
pub fn separator_width(text: &str, terminal_width: u16) -> usize {
    let term_w = terminal_width as usize;
    let text_width = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);

    if text_width > term_w {
        term_w
    } else {
        text_width.max(MIN_SEPARATOR_WIDTH).min(term_w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_returns_minimum() {
        assert_eq!(separator_width("hi", 80), MIN_SEPARATOR_WIDTH);
    }

    #[test]
    fn text_exactly_minimum_width() {
        let text = "a".repeat(MIN_SEPARATOR_WIDTH);
        assert_eq!(separator_width(&text, 80), MIN_SEPARATOR_WIDTH);
    }

    #[test]
    fn text_wider_than_minimum_but_fits_terminal() {
        let text = "a".repeat(60);
        assert_eq!(separator_width(&text, 80), 60);
    }

    #[test]
    fn text_exceeds_terminal_width() {
        let text = "a".repeat(100);
        assert_eq!(separator_width(&text, 80), 80);
    }

    #[test]
    fn empty_text_returns_minimum() {
        assert_eq!(separator_width("", 80), MIN_SEPARATOR_WIDTH);
    }

    #[test]
    fn multiline_uses_longest_line() {
        let text = "short\nthis is a longer line with exactly sixty characters in it, yep!!";
        let longest = text.lines().map(|l| l.chars().count()).max().unwrap();
        assert_eq!(separator_width(text, 80), longest);
    }

    #[test]
    fn terminal_narrower_than_minimum() {
        assert_eq!(separator_width("hi", 30), 30);
    }
}
