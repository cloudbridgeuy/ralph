use crate::ansi::{bg, fg, BOLD, RESET};
use crate::status_theme::StatusColors;
use crate::theme_slots::ResolveColors;
use ralph_core::chat_status::Status;
use ralph_core::status_segments::{bar, Context, Separator, Span};
use ralph_core::stream::IterationMetadata;
use ralph_server::config::theme::StatusTheme;
use ralph_server::config::ChatSection;
use ralph_server::watches::locked;
use std::sync::{Arc, Mutex};

pub struct StatusBar {
    colors: StatusColors,
    context: Context,
    separator: Separator,
    color: bool,
    identity: Arc<Mutex<IterationMetadata>>,
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

pub struct BarContext<'a> {
    pub name: &'a str,
    pub thread: Option<&'a str>,
    pub model_args: &'a [String],
    pub identity: Arc<Mutex<IterationMetadata>>,
}

fn model_alias(model_args: &[String]) -> Option<String> {
    model_args
        .windows(2)
        .find(|pair| pair[0] == "--model")
        .map(|pair| pair[1].clone())
}

impl StatusBar {
    pub fn new(theme: &StatusTheme, chat: &ChatSection, color: bool, context: &BarContext) -> Self {
        Self {
            colors: theme.resolve().0,
            context: Context {
                name: context.name.to_string(),
                thread: context.thread.map(str::to_string),
                alias: model_alias(context.model_args),
                ..Context::default()
            },
            separator: separator_for(color, chat.status_powerline),
            color,
            identity: context.identity.clone(),
        }
    }

    pub fn colored(&self) -> bool {
        self.color
    }

    pub fn line(&self, status: &Status, width: usize) -> String {
        let context = Context {
            resolved: locked(&self.identity).model.clone(),
            ..self.context.clone()
        };
        let spans = bar(status, &context, width, self.separator);
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
        StatusBar::new(&StatusTheme::default(), &chat, color, &context(None, &[]))
    }

    fn context<'a>(thread: Option<&'a str>, model_args: &'a [String]) -> BarContext<'a> {
        BarContext {
            name: "ralph-chat",
            thread,
            model_args,
            identity: Arc::default(),
        }
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
        let bar = StatusBar::new(&theme, &chat, true, &context(None, &[]));
        assert!(bar.line(&status(), 40).starts_with("\x1b[101m"));
    }

    #[test]
    fn plain_render_has_no_escapes_and_no_trailing_padding() {
        let shown = status_bar(false, false).line(&status(), 80);
        assert!(!shown.contains('\x1b'));
        assert!(!shown.ends_with(' '));
        assert!(shown.starts_with(" INBOX │ ralph-chat │ 📬 1 unread"));
    }

    #[test]
    fn a_thread_chat_shows_the_session_name_then_the_thread() {
        let bar = StatusBar::new(
            &StatusTheme::default(),
            &ChatSection::default(),
            false,
            &context(Some("t1"), &[]),
        );
        let shown = bar.line(&status(), 80);
        assert!(
            shown.starts_with(" INBOX │ ralph-chat │ thread t1 │ 📬 1 unread"),
            "{shown}"
        );
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

    #[test]
    fn a_thread_chat_shows_its_model_alias() {
        let args = ["--model".to_string(), "haiku".to_string()];
        let bar = StatusBar::new(
            &StatusTheme::default(),
            &ChatSection::default(),
            false,
            &context(Some("t1"), &args),
        );
        let shown = bar.line(&status(), 80);
        assert!(
            shown.starts_with(" INBOX │ ralph-chat │ thread t1 │ haiku │ 📬 1 unread"),
            "{shown}"
        );
    }

    #[test]
    fn main_chat_shows_no_model_before_a_turn() {
        let shown = status_bar(false, false).line(&status(), 80);
        assert!(
            shown.starts_with(" INBOX │ ralph-chat │ 📬 1 unread"),
            "{shown}"
        );
        assert_eq!(model_alias(&[]), None);
    }

    #[test]
    fn the_resolved_id_replaces_the_alias_after_a_turn() {
        let args = ["--model".to_string(), "haiku".to_string()];
        let context = context(Some("t1"), &args);
        let bar = StatusBar::new(
            &StatusTheme::default(),
            &ChatSection::default(),
            false,
            &context,
        );
        locked(&context.identity).model = Some("claude-haiku-4-5".to_string());
        let shown = bar.line(&status(), 100);
        assert!(shown.contains("│ claude-haiku-4-5 │"), "{shown}");
        assert!(!shown.contains("│ haiku │"), "{shown}");
    }

    #[test]
    fn main_shows_its_model_only_after_the_first_turn() {
        let context = context(None, &[]);
        let bar = StatusBar::new(
            &StatusTheme::default(),
            &ChatSection::default(),
            false,
            &context,
        );
        assert!(!bar.line(&status(), 100).contains("claude"));
        locked(&context.identity).model = Some("claude-opus-5".to_string());
        assert!(bar
            .line(&status(), 100)
            .starts_with(" INBOX │ ralph-chat │ claude-opus-5 │"));
    }
}
