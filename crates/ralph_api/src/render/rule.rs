pub fn rule() -> String {
    "<r-rule role=\"separator\"></r-rule>".to_string()
}

#[cfg(test)]
mod tests {
    use super::rule;
    use crate::render::golden::assert_golden;

    #[test]
    fn plain_rule() {
        assert_golden("rule_plain", &rule());
    }
}
