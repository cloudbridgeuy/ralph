use crate::render::escape;

fn line(class: &str, text: &str) -> String {
    format!("<span class=\"{class}\">{}</span>", escape(text))
}

fn state(what: &str, next: Option<&str>, compact: bool) -> String {
    let attribute = if compact { " compact" } else { "" };
    let next = next.map(|text| line("next", text)).unwrap_or_default();
    format!("<r-empty{attribute}>{}{next}</r-empty>", line("what", what))
}

pub fn empty(what: &str) -> String {
    state(what, None, false)
}

pub fn empty_next(what: &str, next: &str) -> String {
    state(what, Some(next), false)
}

pub fn empty_compact(what: &str) -> String {
    state(what, None, true)
}

#[cfg(test)]
mod tests {
    use super::{empty, empty_compact, empty_next};
    use crate::render::golden::{assert_golden, hostile_text, long_text};

    #[test]
    fn plain_message() {
        assert_golden("empty_plain", &empty("No sessions yet"));
    }

    #[test]
    fn hostile_text_is_escaped() {
        assert_golden("empty_hostile", &empty(hostile_text()));
    }

    #[test]
    fn very_long_text() {
        assert_golden("empty_very_long", &empty(&long_text()));
    }

    #[test]
    fn next_line_follows_what() {
        assert_golden("empty_next", &empty_next("No sessions yet", "Start a chat"));
    }

    #[test]
    fn hostile_next_is_escaped() {
        let html = empty_next("x", hostile_text());
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn compact_form_is_marked() {
        assert_golden("empty_compact", &empty_compact("Nothing to show"));
    }
}
