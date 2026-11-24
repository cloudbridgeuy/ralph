use crate::render::{escape, pill_with, PillShape, PillVariant, Tone};

#[derive(Debug, Default, Clone, Copy)]
pub struct ErrorParts<'a> {
    pub what: &'a str,
    pub why: Option<&'a str>,
    pub status: Option<u16>,
    pub next: Option<&'a str>,
    pub compact: bool,
}

fn line(class: &str, inner: &str) -> String {
    format!("<span class=\"{class}\">{inner}</span>")
}

fn status_pill(status: u16) -> String {
    pill_with(
        PillVariant::Outline,
        Tone::Bad,
        PillShape::Square,
        None,
        &status.to_string(),
    )
}

pub fn error_with(parts: &ErrorParts) -> String {
    let attribute = if parts.compact { " compact" } else { "" };
    let status = parts.status.map(status_pill).unwrap_or_default();
    let what = line("what", &format!("{}{status}", escape(parts.what)));
    let why = parts
        .why
        .map(|text| line("why", &escape(text)))
        .unwrap_or_default();
    let next = parts
        .next
        .map(|text| line("next", &escape(text)))
        .unwrap_or_default();
    format!("<r-error role=\"alert\"{attribute}>{what}{why}{next}</r-error>")
}

pub fn error(text: &str) -> String {
    error_with(&ErrorParts {
        what: text,
        ..ErrorParts::default()
    })
}

#[cfg(test)]
mod tests {
    use super::{error, error_with, ErrorParts};
    use crate::render::golden::{assert_golden, hostile_text, long_text};

    #[test]
    fn plain_message() {
        assert_golden("error_plain", &error("could not read store"));
    }

    #[test]
    fn hostile_text_is_escaped() {
        assert_golden("error_hostile", &error(hostile_text()));
    }

    #[test]
    fn very_long_text() {
        assert_golden("error_very_long", &error(&long_text()));
    }

    #[test]
    fn full_form_has_what_why_status_and_next() {
        let html = error_with(&ErrorParts {
            what: "Could not load the session",
            why: Some("store is locked"),
            status: Some(503),
            next: Some("Retry"),
            compact: false,
        });
        assert_golden("error_full", &html);
    }

    #[test]
    fn compact_form_is_marked_and_keeps_the_alert_role() {
        let html = error_with(&ErrorParts {
            what: "Could not load",
            why: Some("timeout"),
            next: Some("Retry"),
            compact: true,
            ..ErrorParts::default()
        });
        assert_golden("error_compact", &html);
    }

    #[test]
    fn hostile_why_is_escaped() {
        let html = error_with(&ErrorParts {
            what: "Failed",
            why: Some("<script>\"x\"'&"),
            ..ErrorParts::default()
        });
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;"));
        assert_golden("error_hostile_why", &html);
    }

    #[test]
    fn absent_parts_leave_no_empty_elements() {
        let html = error("only what");
        assert!(!html.contains("class=\"why\""));
        assert!(!html.contains("class=\"next\""));
        assert!(!html.contains("r-pill"));
    }
}
