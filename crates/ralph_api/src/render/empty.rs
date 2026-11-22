use crate::render::escape;

pub fn empty(text: &str) -> String {
    format!("<r-empty>{}</r-empty>", escape(text))
}

#[cfg(test)]
mod tests {
    use super::empty;
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
}
