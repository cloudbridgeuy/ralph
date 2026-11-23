use pulldown_cmark::LinkType;

pub fn link_dest(link_type: LinkType, dest: &str) -> String {
    match link_type {
        LinkType::Email => format!("mailto:{dest}"),
        _ => dest.to_string(),
    }
}

pub fn safe_href(dest: &str) -> Option<String> {
    let href: String = dest
        .trim_matches(|c: char| c.is_control() || c.is_whitespace())
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            if c.is_whitespace() {
                "%20".to_string()
            } else {
                c.to_string()
            }
        })
        .collect();
    let lower = href.to_ascii_lowercase();
    let allowed = ["http://", "https://", "mailto:", "#"]
        .iter()
        .any(|prefix| lower.starts_with(prefix));
    allowed.then_some(href)
}

#[cfg(test)]
mod tests {
    use super::{link_dest, safe_href};
    use pulldown_cmark::LinkType;

    #[test]
    fn email_links_get_a_mailto_scheme() {
        assert_eq!(link_dest(LinkType::Email, "a@b.c"), "mailto:a@b.c");
        assert_eq!(link_dest(LinkType::Inline, "a@b.c"), "a@b.c");
        assert_eq!(link_dest(LinkType::Autolink, "http://a.b"), "http://a.b");
    }

    #[test]
    fn allows_http_https_mailto_and_fragments() {
        for url in ["http://a.b/c", "https://a.b", "mailto:a@b.c", "#top"] {
            assert_eq!(safe_href(url).as_deref(), Some(url));
        }
    }

    #[test]
    fn drops_script_and_data_schemes() {
        for url in [
            "javascript:alert(1)",
            "data:text/html,x",
            "vbscript:x",
            "file:///etc/passwd",
        ] {
            assert_eq!(safe_href(url), None);
        }
    }

    #[test]
    fn ignores_case_when_checking_the_scheme() {
        assert_eq!(safe_href("JavaScript:x"), None);
        assert_eq!(safe_href("HTTPS://a.b").as_deref(), Some("HTTPS://a.b"));
    }

    #[test]
    fn strips_control_characters_and_whitespace_before_checking() {
        assert_eq!(safe_href("java\tscript:x"), None);
        assert_eq!(safe_href("\u{1}javascript:x"), None);
        assert_eq!(safe_href(" http://a.b").as_deref(), Some("http://a.b"));
        assert_eq!(safe_href("ht\ntp://a.b").as_deref(), Some("http://a.b"));
    }

    #[test]
    fn inner_spaces_are_encoded_not_deleted() {
        assert_eq!(
            safe_href("https://ex.com/a b").as_deref(),
            Some("https://ex.com/a%20b")
        );
        assert_eq!(
            safe_href(" https://ex.com/a  b ").as_deref(),
            Some("https://ex.com/a%20%20b")
        );
        assert_eq!(
            safe_href("https://ex.com/a\u{a0}b").as_deref(),
            Some("https://ex.com/a%20b")
        );
    }

    #[test]
    fn a_space_inside_the_scheme_is_dropped() {
        assert_eq!(safe_href("java script:x"), None);
        assert_eq!(safe_href("ht tp://a.b"), None);
    }

    #[test]
    fn drops_relative_paths_and_empty_values() {
        assert_eq!(safe_href("/path"), None);
        assert_eq!(safe_href("page.html"), None);
        assert_eq!(safe_href(""), None);
    }
}
