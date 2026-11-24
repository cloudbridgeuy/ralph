use crate::render::escape;

pub fn spinner(label: &str) -> String {
    format!(
        "<r-spinner role=\"status\"><span>{}</span></r-spinner>",
        escape(spinner_label(label))
    )
}

fn spinner_label(label: &str) -> &str {
    match label.trim() {
        "" => "Loading",
        named => named,
    }
}

#[cfg(test)]
mod tests {
    use super::{spinner, spinner_label};
    use crate::render::golden::{assert_golden, hostile_text};

    #[test]
    fn plain_spinner() {
        assert_golden("spinner_plain", &spinner("Loading memory"));
    }

    #[test]
    fn hostile_label_is_escaped() {
        assert_golden("spinner_hostile", &spinner(hostile_text()));
    }

    #[test]
    fn empty_label_falls_back_to_loading() {
        assert_golden("spinner_empty", &spinner(""));
    }

    #[test]
    fn blank_label_falls_back_to_loading() {
        assert_eq!(spinner_label("  \t"), "Loading");
        assert_eq!(spinner_label(" memory "), "memory");
    }

    #[test]
    fn spinner_has_no_inline_style() {
        assert!(!spinner("Loading memory").contains("style="));
    }

    #[test]
    fn spinner_label_is_visible_text() {
        assert!(!spinner("Loading memory").contains("sr-only"));
    }
}
