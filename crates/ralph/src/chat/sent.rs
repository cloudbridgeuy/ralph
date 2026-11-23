use ralph_core::chat_command::leading_command;
use reedline::{Color, Highlighter, StyledText};

#[derive(Clone, Copy, Default)]
pub(crate) struct SentStyle {
    color: Option<Color>,
}

impl SentStyle {
    pub(crate) fn new(color: Option<Color>) -> Self {
        Self { color }
    }

    pub(super) fn paint(self, text: &str) -> String {
        self.highlight(text, text.len()).render_simple()
    }

    pub(super) fn style_leading(self, styled: &mut StyledText, line: &str) {
        if let (Some(color), Some(range)) = (self.color, leading_command(line)) {
            styled.style_range(range.start, range.end, color.bold());
        }
    }
}

impl Highlighter for SentStyle {
    fn highlight(&self, line: &str, _: usize) -> StyledText {
        let mut styled = StyledText::new();
        styled.push((Default::default(), line.to_string()));
        self.style_leading(&mut styled, line);
        styled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_style_keeps_arguments_and_unknown_command_names() {
        let color = Color::Rgb(17, 34, 51);
        let style = SentStyle::new(Some(color));
        for name in ["/hold", "/send-it", "/merge", "/Custom_12-name"] {
            let text = format!(" \t{name} GUZ-1457\n/merge");
            let styled = style.highlight(&text, 0);
            assert_eq!(styled.raw_string(), text);
            assert_eq!(
                styled.buffer,
                vec![
                    (Default::default(), " \t".to_string()),
                    (color.bold(), name.to_string()),
                    (Default::default(), " GUZ-1457\n/merge".to_string()),
                ]
            );
            assert_eq!(
                style.paint(&text),
                format!(" \t{} GUZ-1457\n/merge", color.bold().paint(name))
            );
        }
    }

    #[test]
    fn submit_style_highlights_leading_worker_addresses() {
        let color = Color::Rgb(17, 34, 51);
        let style = SentStyle::new(Some(color));
        for address in ["@GUZ-1449", "@w1", "@thread:x"] {
            let text = format!(" \t{address} run tests\n@w2");
            let styled = style.highlight(&text, 0);
            assert_eq!(styled.raw_string(), text);
            assert_eq!(
                styled.buffer,
                vec![
                    (Default::default(), " \t".to_string()),
                    (color.bold(), address.to_string()),
                    (Default::default(), " run tests\n@w2".to_string()),
                ]
            );
            assert_eq!(
                style.paint(&text),
                format!(" \t{} run tests\n@w2", color.bold().paint(address))
            );
        }
    }

    #[test]
    fn submit_style_leaves_non_commands_and_color_disabled_text_unchanged() {
        let style = SentStyle::new(Some(Color::Magenta));
        for text in [
            "/tmp/x",
            "//x",
            "/",
            "text /hold",
            "/hold!",
            "/1name",
            "@",
            "@ hi",
            "@@GUZ-1 hi",
            "me@example.com",
            "hi @w1",
            "hello",
            "",
        ] {
            assert_eq!(style.paint(text), text);
        }
        assert_eq!(
            SentStyle::new(None).paint(" /hold GUZ-1457"),
            " /hold GUZ-1457"
        );
        assert_eq!(SentStyle::new(None).paint(" @w1 hi"), " @w1 hi");
    }
}
