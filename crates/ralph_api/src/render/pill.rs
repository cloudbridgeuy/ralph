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
pub enum Icon {
    Check,
    Cross,
    Warning,
    Info,
}

impl Icon {
    fn path(self) -> &'static str {
        match self {
            Icon::Check => "M3 8L6.5 11.5L13 4.5",
            Icon::Cross => "M4 4L12 12M12 4L4 12",
            Icon::Warning => "M8 2L15 14H1ZM8 6V9M8 11.5V12",
            Icon::Info => "M8 7V12M8 4V4.5",
        }
    }
}

fn icon_svg(icon: Icon) -> String {
    format!(
        "<svg class=\"icon\" aria-hidden=\"true\" focusable=\"false\" viewBox=\"0 0 16 16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"{}\"/></svg>",
        icon.path()
    )
}

pub fn pill(tone: Tone, shape: PillShape, icon: Option<Icon>, text: &str) -> String {
    format!(
        "<r-pill tone=\"{}\" shape=\"{}\">{}<span>{}</span></r-pill>",
        tone.as_str(),
        shape.as_str(),
        icon.map(icon_svg).unwrap_or_default(),
        escape(text)
    )
}

#[cfg(test)]
mod tests {
    use super::{pill, Icon, PillShape};
    use crate::render::golden::{assert_golden, hostile_text, long_text};
    use crate::render::Tone;

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
        for icon in [Icon::Check, Icon::Cross, Icon::Warning, Icon::Info] {
            let html = pill(Tone::Ok, PillShape::Round, Some(icon), "x");
            assert!(html.contains("aria-hidden=\"true\""), "{html}");
            assert!(!html.contains("style="), "{html}");
        }
    }

    #[test]
    fn icon_paths_differ() {
        let paths: Vec<&str> = [Icon::Check, Icon::Cross, Icon::Warning, Icon::Info]
            .into_iter()
            .map(Icon::path)
            .collect();
        let mut unique = paths.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), paths.len());
    }
}
