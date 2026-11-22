use crate::render::{escape, Tone};

pub fn status(tone: Tone, text: &str) -> String {
    format!(
        "<r-status tone=\"{}\" role=\"status\" aria-live=\"polite\"><span class=\"mark\">{}:</span> {}</r-status>",
        tone.as_str(),
        tone.label(),
        escape(text)
    )
}

#[cfg(test)]
mod tests {
    use super::status;
    use crate::render::golden::{assert_golden, hostile_text, long_text};
    use crate::render::Tone;

    #[test]
    fn ok_status() {
        assert_golden("status_ok", &status(Tone::Ok, "all good"));
    }

    #[test]
    fn bad_status() {
        assert_golden("status_bad", &status(Tone::Bad, "disk full"));
    }

    #[test]
    fn empty_text() {
        assert_golden("status_empty", &status(Tone::Warn, ""));
    }

    #[test]
    fn hostile_text_is_escaped() {
        assert_golden("status_hostile", &status(Tone::Info, hostile_text()));
    }

    #[test]
    fn very_long_text() {
        assert_golden("status_very_long", &status(Tone::Accent, &long_text()));
    }

    #[test]
    fn every_tone_carries_a_word_not_only_a_colour() {
        for tone in [Tone::Ok, Tone::Bad, Tone::Warn, Tone::Info, Tone::Accent] {
            let html = status(tone, "x");
            assert!(
                html.contains(&format!("{}:</span>", tone.label())),
                "{html}"
            );
        }
    }
}
