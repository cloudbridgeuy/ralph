use crate::render::{escape, Tone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillShape {
    Square,
    Round,
}

impl PillShape {
    fn as_str(self) -> &'static str {
        match self {
            PillShape::Square => "square",
            PillShape::Round => "round",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillVariant {
    Tint,
    Outline,
}

impl PillVariant {
    fn attribute(self) -> &'static str {
        match self {
            PillVariant::Tint => "",
            PillVariant::Outline => " variant=\"outline\"",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Check,
    Cross,
    Warning,
    Info,
    Dot,
    Pause,
}

impl Icon {
    fn path(self) -> &'static str {
        match self {
            Icon::Check => "M3 8L6.5 11.5L13 4.5",
            Icon::Cross => "M4 4L12 12M12 4L4 12",
            Icon::Warning => "M8 2L15 14H1ZM8 6V9M8 11.5V12",
            Icon::Info => "M8 7V12M8 4V4.5",
            Icon::Dot => "M8 5.5A2.5 2.5 0 1 1 8 10.5A2.5 2.5 0 1 1 8 5.5Z",
            Icon::Pause => "M6 4V12M10 4V12",
        }
    }
}

fn icon_svg(icon: Icon) -> String {
    format!(
        "<svg class=\"icon\" aria-hidden=\"true\" focusable=\"false\" viewBox=\"0 0 16 16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"{}\"/></svg>",
        icon.path()
    )
}

/// The CSS cuts a pill at 32ch, so only longer text needs its full form in a tooltip.
const CUT_AT_CHARS: usize = 32;

fn title_attribute(text: &str) -> String {
    if text.chars().count() > CUT_AT_CHARS {
        format!(" title=\"{}\"", escape(text))
    } else {
        String::new()
    }
}

pub fn pill(tone: Tone, shape: PillShape, icon: Option<Icon>, text: &str) -> String {
    pill_with(PillVariant::Tint, tone, shape, icon, text)
}

pub fn pill_with(
    variant: PillVariant,
    tone: Tone,
    shape: PillShape,
    icon: Option<Icon>,
    text: &str,
) -> String {
    format!(
        "<r-pill tone=\"{}\" shape=\"{}\"{}{}>{}<span>{}</span></r-pill>",
        tone.as_str(),
        shape.as_str(),
        variant.attribute(),
        title_attribute(text),
        icon.map(icon_svg).unwrap_or_default(),
        escape(text)
    )
}

#[cfg(test)]
mod tests {
    use super::{pill, pill_with, Icon, PillShape, PillVariant};
    use crate::render::golden::{assert_golden, hostile_text, long_text};
    use crate::render::Tone;

    const ALL_ICONS: [Icon; 6] = [
        Icon::Check,
        Icon::Cross,
        Icon::Warning,
        Icon::Info,
        Icon::Dot,
        Icon::Pause,
    ];

    #[test]
    fn plain_pill() {
        assert_golden(
            "pill_plain",
            &pill(Tone::Ok, PillShape::Round, None, "idle"),
        );
    }

    #[test]
    fn failed_pill_with_cross_icon() {
        assert_golden(
            "pill_icon_bad",
            &pill(Tone::Bad, PillShape::Square, Some(Icon::Cross), "failed"),
        );
    }

    #[test]
    fn done_pill_with_check_icon() {
        assert_golden(
            "pill_icon_ok",
            &pill(Tone::Ok, PillShape::Round, Some(Icon::Check), "done"),
        );
    }

    #[test]
    fn idle_pill_with_dot_icon() {
        assert_golden(
            "pill_icon_dot",
            &pill(Tone::Neutral, PillShape::Round, Some(Icon::Dot), "idle"),
        );
    }

    #[test]
    fn paused_pill_with_pause_icon() {
        assert_golden(
            "pill_icon_pause",
            &pill(Tone::Warn, PillShape::Round, Some(Icon::Pause), "paused"),
        );
    }

    #[test]
    fn long_text_is_repeated_in_title() {
        let text = long_text();
        let html = pill(Tone::Accent, PillShape::Square, None, &text);
        assert!(html.contains(&format!("title=\"{text}\"")), "{html}");
    }

    #[test]
    fn long_hostile_text_is_escaped_in_title() {
        let text = hostile_text().repeat(2);
        assert_golden(
            "pill_long_hostile",
            &pill(Tone::Warn, PillShape::Round, None, &text),
        );
    }

    #[test]
    fn title_starts_past_32_chars_not_bytes() {
        let at_limit = "é".repeat(32);
        let past = "é".repeat(33);
        assert!(!pill(Tone::Ok, PillShape::Round, None, &at_limit).contains("title"));
        assert!(pill(Tone::Ok, PillShape::Round, None, &past).contains("title="));
        assert!(!pill(Tone::Ok, PillShape::Round, None, "idle").contains("title"));
    }

    #[test]
    fn outline_pill() {
        assert_golden(
            "pill_outline",
            &pill_with(
                PillVariant::Outline,
                Tone::Info,
                PillShape::Round,
                Some(Icon::Info),
                "queued",
            ),
        );
    }

    #[test]
    fn tint_is_the_default_and_has_no_variant_attribute() {
        let html = pill(Tone::Ok, PillShape::Round, None, "x");
        assert!(!html.contains("variant"), "{html}");
    }

    #[test]
    fn empty_text() {
        assert_golden("pill_empty", &pill(Tone::Info, PillShape::Square, None, ""));
    }

    #[test]
    fn hostile_text_is_escaped() {
        assert_golden(
            "pill_hostile",
            &pill(Tone::Warn, PillShape::Round, None, hostile_text()),
        );
    }

    #[test]
    fn very_long_text() {
        assert_golden(
            "pill_very_long",
            &pill(Tone::Accent, PillShape::Square, None, &long_text()),
        );
    }

    #[test]
    fn every_icon_is_hidden_from_assistive_tech() {
        for icon in ALL_ICONS {
            let html = pill(Tone::Ok, PillShape::Round, Some(icon), "x");
            assert!(html.contains("aria-hidden=\"true\""), "{html}");
            assert!(!html.contains("style="), "{html}");
        }
    }

    #[test]
    fn icon_paths_differ() {
        let paths: Vec<&str> = ALL_ICONS.into_iter().map(Icon::path).collect();
        let mut unique = paths.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), paths.len());
    }
}
