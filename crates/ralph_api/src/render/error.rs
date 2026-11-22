use crate::render::escape;

pub fn error(text: &str) -> String {
    format!("<r-error role=\"alert\">{}</r-error>", escape(text))
}

#[cfg(test)]
mod tests {
    use super::error;
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
}
