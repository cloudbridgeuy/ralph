use crate::chat::sent::SentStyle;
use ralph_server::chat::suggestion::SuggestionCell;
use reedline::{Color, ExampleHighlighter, Highlighter, StyledText};

pub(crate) struct SuggestionHighlighter {
    shown: SuggestionCell,
    color: Color,
    sent: SentStyle,
    plain: ExampleHighlighter,
}

impl SuggestionHighlighter {
    pub(crate) fn new(shown: SuggestionCell, color: Color, sent: SentStyle) -> Self {
        Self {
            shown,
            color,
            sent,
            plain: ExampleHighlighter::default(),
        }
    }
}

impl Highlighter for SuggestionHighlighter {
    fn highlight(&self, line: &str, cursor: usize) -> StyledText {
        if self.shown.latch_untouched(line) {
            let mut styled = StyledText::new();
            styled.push((self.color.normal(), line.to_string()));
            styled
        } else {
            let mut styled = self.plain.highlight(line, cursor);
            self.sent.style_leading(&mut styled, line);
            styled
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        text.to_string()
    }

    fn highlighter(text: &str) -> (SuggestionHighlighter, SuggestionCell) {
        let shown = SuggestionCell::default();
        shown.set(plain(text));
        (
            SuggestionHighlighter::new(shown.clone(), Color::Cyan, SentStyle::default()),
            shown,
        )
    }

    #[test]
    fn an_untouched_suggestion_is_one_segment_in_the_suggestion_color() {
        let (highlighter, shown) = highlighter("yes, go");
        let styled = highlighter.highlight("yes, go", 7);
        assert_eq!(styled.buffer.len(), 1);
        assert_eq!(styled.buffer[0].0, Color::Cyan.normal());
        assert_eq!(styled.buffer[0].1, "yes, go");
        assert_eq!(shown.take(), Some(plain("yes, go")));
    }

    #[test]
    fn other_text_uses_the_plain_highlighter() {
        let (highlighter, _) = highlighter("yes, go");
        let expected = ExampleHighlighter::default().highlight("mine", 4);
        assert_eq!(highlighter.highlight("mine", 4).buffer, expected.buffer);
    }

    #[test]
    fn the_first_edit_latches_to_plain_even_when_the_text_comes_back() {
        let (highlighter, shown) = highlighter("yes, go");
        let expected = ExampleHighlighter::default().highlight("yes, go", 7);
        highlighter.highlight("yes, g", 6);
        assert_eq!(shown.take(), None);
        assert_eq!(highlighter.highlight("yes, go", 7).buffer, expected.buffer);
    }

    #[test]
    fn an_empty_slot_is_plain() {
        let highlighter = SuggestionHighlighter::new(
            SuggestionCell::default(),
            Color::Cyan,
            SentStyle::default(),
        );
        let expected = ExampleHighlighter::default().highlight("x", 1);
        assert_eq!(highlighter.highlight("x", 1).buffer, expected.buffer);
    }

    #[test]
    fn draft_highlights_leading_command_and_worker_address_when_not_an_untouched_suggestion() {
        let highlighter = SuggestionHighlighter::new(
            SuggestionCell::default(),
            Color::Cyan,
            SentStyle::new(Some(Color::Magenta)),
        );
        let plain_style = ExampleHighlighter::default().highlight("x", 1).buffer[0].0;
        for (line, token, rest) in [
            ("/hold GUZ-1", "/hold", " GUZ-1"),
            ("@w1 run tests", "@w1", " run tests"),
        ] {
            let styled = highlighter.highlight(line, line.len());
            assert_eq!(
                styled.buffer,
                vec![
                    (Color::Magenta.bold(), token.to_string()),
                    (plain_style, rest.to_string()),
                ]
            );
        }
    }
}
