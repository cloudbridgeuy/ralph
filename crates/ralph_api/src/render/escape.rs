pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn escapes_the_five_html_characters() {
        assert_eq!(escape("& < > \" '"), "&amp; &lt; &gt; &quot; &#39;");
    }

    #[test]
    fn neutralises_a_script_tag() {
        assert_eq!(
            escape("<script>alert(1)</script>"),
            "&lt;script&gt;alert(1)&lt;/script&gt;"
        );
    }

    #[test]
    fn neutralises_an_attribute_breakout() {
        assert_eq!(
            escape("\" onmouseover=\"x\" '"),
            "&quot; onmouseover=&quot;x&quot; &#39;"
        );
    }

    #[test]
    fn escapes_an_ampersand_once() {
        assert_eq!(escape("&amp;"), "&amp;amp;");
    }

    #[test]
    fn keeps_unicode_and_plain_text() {
        assert_eq!(escape("café 日本 🦀 plain"), "café 日本 🦀 plain");
    }

    #[test]
    fn keeps_the_empty_string() {
        assert_eq!(escape(""), "");
    }
}
