use crate::ansi::{bg, fg, BOLD, RESET};
use crate::config::ChatSection;
use crate::status_theme::{StatusColors, StatusTheme};
use ralph_core::chat_status::Status;
use ralph_core::status_segments::{bar, store_label, Context, Separator, Span};
use std::path::Path;

pub struct StatusBar {
    colors: StatusColors,
    context: Context,
    separator: Separator,
    color: bool,
}

fn separator_for(color: bool, powerline: bool) -> Separator {
    match (color, powerline) {
        (false, _) => Separator::Bar,
        (true, true) => Separator::Powerline,
        (true, false) => Separator::Block,
    }
}

fn paint(colors: &StatusColors, span: &Span) -> String {
    let weight = if span.bold { BOLD } else { "" };
    format!(
        "{}{}{weight}{}{RESET}",
        bg(Some(colors.color(span.bg))),
        fg(Some(colors.color(span.fg))),
        span.text
    )
}

fn render_spans(colors: &StatusColors, spans: &[Span], color: bool) -> String {
    if color {
        spans.iter().map(|span| paint(colors, span)).collect()
    } else {
        let plain: String = spans.iter().map(|span| span.text.as_str()).collect();
        plain.trim_end().to_string()
    }
}

impl StatusBar {
    pub fn new(theme: &StatusTheme, store: &Path, chat: &ChatSection, color: bool) -> Self {
        Self {
            colors: theme.resolve().0,
            context: Context {
                store: store_label(&store.to_string_lossy()),
            },
            separator: separator_for(color, chat.status_powerline),
            color,
        }
    }

    pub fn line(&self, status: &Status, width: usize) -> String {
        let spans = bar(status, &self.context, width, self.separator);
        format!("{}\n", render_spans(&self.colors, &spans, self.color))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::strip_ansi;
    use ralph_core::status_segments::Shade;

    fn status() -> Status {
        Status {
            unread: 1,
            ..Status::default()
        }
    }

    fn status_bar(color: bool, powerline: bool) -> StatusBar {
        let chat = ChatSection {
            status_powerline: powerline,
            ..ChatSection::default()
        };
        StatusBar::new(
            &StatusTheme::default(),
            Path::new("/home/me/.ralph/chat"),
            &chat,
            color,
        )
    }

    #[test]
    fn separator_is_a_plain_rule_without_color() {
        assert_eq!(separator_for(false, true), Separator::Bar);
        assert_eq!(separator_for(false, false), Separator::Bar);
        assert_eq!(separator_for(true, false), Separator::Block);
        assert_eq!(separator_for(true, true), Separator::Powerline);
    }

    #[test]
    fn paint_sets_background_foreground_and_weight() {
        let span = Span {
            text: " IDLE ".into(),
            fg: Shade::ModeIdleFg,
            bg: Shade::ModeIdleBg,
            bold: true,
        };
        assert_eq!(
            paint(&StatusColors::default(), &span),
            "\x1b[102m\x1b[30m\x1b[1m IDLE \x1b[0m"
        );
    }

    #[test]
    fn theme_overrides_reach_the_painted_output() {
        let theme = StatusTheme {
            colors: [("mode_inbox_bg".to_string(), "red".to_string())].into(),
        };
        let chat = ChatSection::default();
        let bar = StatusBar::new(&theme, Path::new("/s"), &chat, true);
        assert!(bar.line(&status(), 40).starts_with("\x1b[101m"));
    }

    #[test]
    fn plain_render_has_no_escapes_and_no_trailing_padding() {
        let shown = status_bar(false, false).line(&status(), 80);
        assert!(!shown.contains('\x1b'));
        assert!(!shown.ends_with(' '));
        assert!(shown.starts_with(" INBOX │ 📬 1 unread │ .ralph/chat"));
    }

    #[test]
    fn colored_render_paints_every_span_and_fits_the_width() {
        let shown = status_bar(true, false).line(&status(), 40);
        assert!(shown.contains("\x1b[0m"));
        let stripped = strip_ansi(shown.trim_end_matches('\n'));
        assert_eq!(unicode_width::UnicodeWidthStr::width(stripped.as_str()), 40);
    }

    #[test]
    fn powerline_flag_adds_glyphs_only_with_color() {
        let glyph = "\u{e0b0}";
        assert!(status_bar(true, true).line(&status(), 80).contains(glyph));
        assert!(!status_bar(true, false).line(&status(), 80).contains(glyph));
        assert!(!status_bar(false, true).line(&status(), 80).contains(glyph));
    }
}
