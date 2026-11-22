pub fn spinner() -> String {
    "<r-spinner role=\"status\"><span class=\"sr-only\">Loading</span></r-spinner>".to_string()
}
#[cfg(test)]
mod tests {
    use super::spinner;
    use crate::render::golden::assert_golden;

    #[test]
    fn plain_spinner() {
        assert_golden("spinner_plain", &spinner());
    }

    #[test]
    fn spinner_has_no_inline_style() {
        assert!(!spinner().contains("style="));
    }
}
