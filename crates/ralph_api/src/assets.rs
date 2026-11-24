pub(crate) struct Asset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

pub(crate) const SHELL_PATH: &str = "/";

const ASSETS: &[Asset] = &[
    Asset {
        path: SHELL_PATH,
        content_type: "text/html; charset=utf-8",
        bytes: include_bytes!("../assets/index.html"),
    },
    Asset {
        path: "/assets/app.css",
        content_type: "text/css; charset=utf-8",
        bytes: include_bytes!("../assets/app.css"),
    },
    Asset {
        path: "/app.js",
        content_type: "text/javascript; charset=utf-8",
        bytes: include_bytes!("../assets/app.js"),
    },
    Asset {
        path: "/assets/geist-mono.woff2",
        content_type: "font/woff2",
        bytes: include_bytes!("../assets/fonts/geist-mono.woff2"),
    },
];

pub(crate) fn find(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|asset| asset.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_of(path: &str) -> &'static [u8] {
        find(path).unwrap().bytes
    }

    #[test]
    fn the_shell_is_html() {
        assert_eq!(find("/").unwrap().content_type, "text/html; charset=utf-8");
        assert!(bytes_of("/").starts_with(b"<!doctype html>"));
    }

    #[test]
    fn the_stylesheet_is_css() {
        assert_eq!(
            find("/assets/app.css").unwrap().content_type,
            "text/css; charset=utf-8"
        );
        assert!(!bytes_of("/assets/app.css").is_empty());
    }

    #[test]
    fn the_font_is_the_upstream_woff2() {
        let font = find("/assets/geist-mono.woff2").unwrap();
        assert_eq!(font.content_type, "font/woff2");
        assert!(font.bytes.starts_with(b"wOF2"));
        assert_eq!(font.bytes.len(), 71_212);
    }

    #[test]
    fn unknown_and_traversal_paths_find_nothing() {
        for path in [
            "/nope",
            "/assets/",
            "/assets/../index.html",
            "/assets/app.css/",
            "",
            "/index.html",
        ] {
            assert!(find(path).is_none(), "{path}");
        }
    }

    #[test]
    fn every_path_is_unique_and_absolute() {
        assert!(!ASSETS.is_empty());
        for (at, asset) in ASSETS.iter().enumerate() {
            assert!(asset.path.starts_with('/'), "{}", asset.path);
            assert!(!asset.bytes.is_empty(), "{}", asset.path);
            assert!(ASSETS[at + 1..]
                .iter()
                .all(|other| other.path != asset.path));
        }
    }

    fn css() -> String {
        String::from_utf8(bytes_of("/assets/app.css").to_vec()).unwrap()
    }

    #[test]
    fn the_stylesheet_styles_every_primitive() {
        let css = css();
        for tag in [
            "r-box",
            "r-rule",
            "r-table",
            "r-kv",
            "r-pill",
            "r-status",
            "r-empty",
            "r-error",
            "r-spinner",
            "r-spark",
        ] {
            assert!(css.contains(&format!("\n{tag}")), "{tag}");
        }
    }

    #[test]
    fn the_stylesheet_has_the_three_box_shapes_and_two_pill_shapes() {
        let css = css();
        for selector in [
            "r-box[shape=\"square\"]",
            "r-box[shape=\"round\"]",
            "r-box[shape=\"heavy\"]",
            "r-pill[shape=\"square\"]",
            "r-pill[shape=\"round\"]",
        ] {
            assert!(css.contains(selector), "{selector}");
        }
    }

    #[test]
    fn the_stylesheet_has_a_rule_for_every_tone() {
        let css = css();
        for tone in ["ok", "bad", "warn", "info", "accent", "neutral"] {
            assert!(css.contains(&format!("r-pill[tone=\"{tone}\"]")), "{tone}");
            assert!(
                css.contains(&format!("r-status[tone=\"{tone}\"]")),
                "{tone}"
            );
        }
    }

    #[test]
    fn the_spinner_shows_after_200ms_from_zero_opacity() {
        let css = css();
        assert!(css.contains("r-spinner::before"));
        assert!(css.contains("200ms both"));
        assert!(css.contains("@keyframes r-show"));
        assert!(css.contains("opacity: 0;"));
    }

    #[test]
    fn reduced_motion_turns_the_spinner_animation_off() {
        let css = css();
        let media = css
            .split("@media (prefers-reduced-motion: reduce)")
            .nth(1)
            .unwrap();
        assert!(media.contains("r-spinner::before"));
        assert!(media.contains("animation: none"));
    }

    #[test]
    fn reduced_motion_keeps_the_spinner_show_delay_and_drops_the_spin() {
        let css = css();
        let media = css
            .split("@media (prefers-reduced-motion: reduce)")
            .nth(1)
            .unwrap();
        let rule = media
            .split("r-spinner::before")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .unwrap();
        assert!(rule.contains("r-show 1ms linear 200ms both"), "{rule}");
        assert!(!rule.contains("r-spin"), "{rule}");
    }

    #[test]
    fn the_stylesheet_has_the_two_theme_columns_and_a_hidden_text_class() {
        let css = css();
        assert!(css.contains(".columns"));
        assert!(css.contains("[data-theme]"));
        assert!(css.contains(".sr-only"));
    }

    #[test]
    fn the_stylesheet_paints_only_with_tokens() {
        let css = css();
        assert!(!css.contains("style="));
        assert!(!css.contains(" #"));
        assert!(!css.contains("rgb(") && !css.contains("rgba("));
    }

    const UNSTYLED_SCOPES: [&str; 2] = ["source", "meta"];

    fn scope_heads(html: &str) -> Vec<String> {
        html.split("<span class=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .filter_map(|classes| classes.split(' ').next())
            .filter(|class| class.starts_with("t-"))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn scope_heads_are_the_first_class_of_each_span() {
        let html = "<span class=\"t-a t-b\">x<span class=\"t-c\">y</span></span><span class=\"u\">";
        assert_eq!(scope_heads(html), vec!["t-a", "t-c"]);
    }

    #[test]
    fn the_stylesheet_maps_every_scope_in_the_rust_sample() {
        let css = std::str::from_utf8(bytes_of("/assets/app.css")).unwrap();
        let sample = include_str!("../tests/golden/render/code_rust.html");
        let heads = scope_heads(sample);
        assert!(!heads.is_empty());
        for head in heads {
            let name = head.trim_start_matches("t-");
            let mapped = css.contains(&format!(".{head}"));
            assert!(
                mapped || UNSTYLED_SCOPES.contains(&name),
                "{head} has no rule"
            );
        }
    }

    #[test]
    fn the_stylesheet_styles_prose_and_code() {
        let css = std::str::from_utf8(bytes_of("/assets/app.css")).unwrap();
        for tag in ["r-prose", "r-code"] {
            assert!(css.contains(tag), "{tag}");
        }
        assert!(css.contains("r-code pre:focus-visible"));
    }

    fn rule_body<'a>(css: &'a str, selector: &str) -> Option<&'a str> {
        let head = format!("\n{selector} {{\n");
        let start = css.find(&head)? + head.len();
        let end = css[start..].find('}')?;
        Some(&css[start..start + end])
    }

    fn declarations<'a>(css: &'a str, property: &str) -> Vec<&'a str> {
        css.lines()
            .map(str::trim)
            .filter(|line| line.starts_with(property))
            .collect()
    }

    #[test]
    fn rule_body_reads_the_block_of_an_exact_selector() {
        let css = "\nbody {\n  color: red;\n}\n\nbody-x {\n  color: blue;\n}\n";
        assert_eq!(rule_body(css, "body"), Some("  color: red;\n"));
        assert_eq!(rule_body(css, "main"), None);
    }

    #[test]
    fn declarations_match_only_the_property_prefix() {
        let css = "a {\n  border: 0;\n  border-top: 1px;\n  outline: none;\n}\n";
        assert_eq!(
            declarations(css, "border"),
            vec!["border: 0;", "border-top: 1px;"]
        );
    }

    #[test]
    fn the_stylesheet_uses_neutral_roles() {
        let css = css();
        for gone in ["--r-border", "--r-link", "--r-inline-code-bg"] {
            assert!(!css.contains(gone), "{gone}");
        }
        let outlines = declarations(&css, "outline:");
        assert!(!outlines.is_empty());
        for outline in outlines {
            assert_eq!(outline, "outline: 2px solid var(--r-accent);");
        }
        for border in declarations(&css, "border") {
            assert!(!border.contains("--r-ansi-"), "{border}");
        }
        let body = rule_body(&css, "body").unwrap();
        assert!(body.contains("background: var(--r-bg);"));
        assert!(body.contains("color: var(--r-ink);"));
        for level in ["h1", "h2", "h3"] {
            let colour = format!("color: var(--r-{level});");
            assert!(css.contains(&colour), "{colour}");
        }
        assert!(!css.contains("var(--r-h1,"));
        assert!(!css.contains("var(--r-h2,"));
        assert!(!css.contains("var(--r-h3,"));
    }
    #[test]
    fn the_neutral_tone_is_muted() {
        let css = css();
        let body = rule_body(&css, "r-pill[tone=\"neutral\"]").unwrap();
        assert!(body.contains("color: var(--r-muted);"), "{body}");
    }

    #[test]
    fn every_pill_tone_fills_with_its_own_role_over_the_page() {
        let css = css();
        for (tone, role) in [
            ("ok", "ok"),
            ("bad", "bad"),
            ("warn", "warn"),
            ("info", "info"),
            ("accent", "accent"),
            ("neutral", "muted"),
        ] {
            let body = rule_body(&css, &format!("r-pill[tone=\"{tone}\"]")).unwrap();
            assert!(
                body.contains(&format!(
                    "background: color-mix(in srgb, var(--r-{role}) 8%, var(--r-bg));"
                )),
                "{tone}: {body}"
            );
            let status = rule_body(&css, &format!("r-status[tone=\"{tone}\"]")).unwrap();
            assert!(!status.contains("background"), "{tone}: {status}");
        }
        assert!(!rule_body(&css, "r-pill").unwrap().contains("background"));
    }

    #[test]
    fn the_outline_pill_has_no_fill_and_follows_every_tone_rule() {
        let css = css();
        let body = rule_body(&css, "r-pill[variant=\"outline\"]").unwrap();
        assert!(body.contains("background: transparent;"), "{body}");
        let outline = css.find("r-pill[variant=\"outline\"]").unwrap();
        let last_tone = css.find("r-pill[tone=\"neutral\"]").unwrap();
        assert!(outline > last_tone);
    }

    #[test]
    fn info_and_accent_pills_use_different_roles() {
        let css = css();
        let info = rule_body(&css, "r-pill[tone=\"info\"]").unwrap();
        let accent = rule_body(&css, "r-pill[tone=\"accent\"]").unwrap();
        assert!(info.contains("var(--r-info)"), "{info}");
        assert!(accent.contains("var(--r-accent)"), "{accent}");
        assert_ne!(info, accent);
    }

    #[test]
    fn faint_and_line_strong_stay_off_text_colours() {
        let css = css();
        let colours = declarations(&css, "color:");
        assert!(!colours.is_empty());
        for colour in colours {
            assert!(!colour.contains("--r-faint"), "{colour}");
            assert!(!colour.contains("--r-line-strong"), "{colour}");
        }
    }
}
