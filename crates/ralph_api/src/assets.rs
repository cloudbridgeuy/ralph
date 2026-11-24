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
        let mut percents = Vec::new();
        for (tone, role) in [
            ("ok", "ok"),
            ("bad", "bad"),
            ("warn", "warn"),
            ("info", "info"),
            ("accent", "accent"),
            ("neutral", "muted"),
        ] {
            let body = rule_body(&css, &format!("r-pill[tone=\"{tone}\"]")).unwrap();
            let prefix = format!("background: color-mix(in srgb, var(--r-{role}) ");
            let from = body
                .find(&prefix)
                .unwrap_or_else(|| panic!("{tone}: {body}"))
                + prefix.len();
            let digits: String = body[from..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            assert!(
                body[from + digits.len()..].starts_with("%, var(--r-bg));"),
                "{tone}: {body}"
            );
            percents.push(digits);
            let status = rule_body(&css, &format!("r-status[tone=\"{tone}\"]")).unwrap();
            assert!(!status.contains("background"), "{tone}: {status}");
        }
        assert!(percents.iter().all(|p| p == &percents[0]), "{percents:?}");
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
    fn a_long_pill_is_cut_with_an_ellipsis_at_32ch() {
        let css = css();
        let pill = rule_body(&css, "r-pill").unwrap();
        assert!(pill.contains("max-width: 100%;"), "{pill}");
        assert!(!pill.contains("32ch"), "{pill}");
        assert!(!pill.contains("overflow-wrap"), "{pill}");
        let text = rule_body(&css, "r-pill > span").unwrap();
        for declaration in [
            "max-width: 32ch;",
            "min-width: 0;",
            "overflow: hidden;",
            "text-overflow: ellipsis;",
            "white-space: nowrap;",
        ] {
            assert!(text.contains(declaration), "{declaration}: {text}");
        }
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

    fn assert_declares(css: &str, selector: &str, declaration: &str) {
        let body = rule_body(css, selector).unwrap_or_else(|| panic!("{selector} has no rule"));
        assert!(body.contains(declaration), "{selector} lacks {declaration}");
    }

    #[test]
    fn the_body_is_13px_on_a_20px_line() {
        let css = css();
        assert_declares(&css, "body", "font-size: 13px;");
        assert_declares(&css, "body", "line-height: 20px;");
    }

    #[test]
    fn meta_text_is_12px() {
        let css = css();
        for selector in [".note", "r-table th", "r-prose th", "r-pill", "r-spark"] {
            assert_declares(&css, selector, "font-size: 12px;");
        }
    }

    #[test]
    fn every_font_size_is_on_the_scale() {
        let css = css();
        let sizes = declarations(&css, "font-size:");
        assert!(!sizes.is_empty());
        for size in sizes {
            assert!(
                ["font-size: 12px;", "font-size: 13px;", "font-size: 1em;"].contains(&size),
                "{size}"
            );
        }
    }

    #[test]
    fn font_weights_stay_on_the_three_step_scale() {
        let css = css();
        let weights = declarations(&css, "font-weight:");
        assert!(!weights.is_empty());
        for weight in weights {
            assert!(
                [
                    "font-weight: 100 900;",
                    "font-weight: 400;",
                    "font-weight: 500;",
                    "font-weight: 600;"
                ]
                .contains(&weight),
                "{weight}"
            );
        }
    }

    #[test]
    fn headers_and_pills_weigh_500() {
        let css = css();
        for selector in ["r-table th", "r-prose th", "r-pill"] {
            assert_declares(&css, selector, "font-weight: 500;");
        }
    }

    #[test]
    fn names_and_titles_weigh_600() {
        let css = css();
        for selector in [
            ".brand",
            "r-box .title",
            "r-status .mark",
            "r-prose strong",
            "r-tree button",
            "r-prose h1,\nr-prose h2,\nr-prose h3,\nr-prose h4,\nr-prose h5,\nr-prose h6",
        ] {
            assert_declares(&css, selector, "font-weight: 600;");
        }
    }

    fn spacing_declarations(css: &str) -> Vec<&str> {
        let mut all = declarations(css, "margin");
        all.extend(declarations(css, "padding"));
        all
    }

    #[test]
    fn spacing_declarations_collect_margins_and_paddings() {
        let css = "a {\n  margin: 0;\n  padding-left: 1ch;\n  color: red;\n}\n";
        assert_eq!(
            spacing_declarations(css),
            vec!["margin: 0;", "padding-left: 1ch;"]
        );
    }

    #[test]
    fn page_gutters_are_2ch() {
        let css = css();
        assert_declares(&css, "main", "padding: 20px 2ch 30px;");
        assert_declares(&css, ".bar,\n.nav", "padding: 0 2ch;");
        assert_declares(&css, "r-box", "padding: 10px 2ch;");
        assert_declares(&css, "r-prose", "max-width: 92ch;");
    }

    #[test]
    fn spacing_has_no_rem() {
        let css = css();
        let spacing = spacing_declarations(&css);
        assert!(!spacing.is_empty());
        for line in spacing {
            assert!(!line.contains("rem"), "{line}");
        }
    }

    const SPACING_GRID: [&str; 4] = ["0", "10px", "20px", "30px"];
    const FINE_PADDING_SELECTORS: [&str; 3] = [
        "r-table th,\nr-table td",
        "r-prose th,\nr-prose td",
        "r-tree li",
    ];

    fn vertical_tokens(line: &str) -> Vec<&str> {
        let Some((property, value)) = line.trim_end_matches(';').split_once(": ") else {
            return Vec::new();
        };
        let values: Vec<&str> = value.split(' ').collect();
        match property {
            "margin" | "padding" => match values.len() {
                1 | 2 => vec![values[0]],
                _ => vec![values[0], values[2]],
            },
            "margin-top"
            | "margin-bottom"
            | "margin-block"
            | "margin-block-start"
            | "margin-block-end"
            | "padding-top"
            | "padding-bottom"
            | "padding-block"
            | "padding-block-start"
            | "padding-block-end" => values,
            _ => Vec::new(),
        }
    }

    fn vertical_violations(css: &str) -> Vec<String> {
        let mut found = Vec::new();
        for chunk in css.split('}') {
            let Some((head, body)) = chunk.rsplit_once('{') else {
                continue;
            };
            let selector = head.trim();
            if selector == ".sr-only" {
                continue;
            }
            for line in body.lines().map(str::trim) {
                let fine =
                    line.starts_with("padding") && FINE_PADDING_SELECTORS.contains(&selector);
                let bad = vertical_tokens(line)
                    .into_iter()
                    .any(|token| !(SPACING_GRID.contains(&token) || fine && token == "2px"));
                if bad {
                    found.push(line.to_string());
                }
            }
        }
        found
    }

    #[test]
    fn vertical_tokens_pick_only_the_vertical_positions() {
        assert_eq!(
            vertical_tokens("margin: 1px 2px 3px 4px;"),
            vec!["1px", "3px"]
        );
        assert_eq!(vertical_tokens("padding: 1px 2px 3px;"), vec!["1px", "3px"]);
        assert_eq!(vertical_tokens("padding: 1px 2px;"), vec!["1px"]);
        assert_eq!(vertical_tokens("margin: 5px;"), vec!["5px"]);
        assert_eq!(
            vertical_tokens("margin-block: 1px 2px;"),
            vec!["1px", "2px"]
        );
        assert!(vertical_tokens("margin-inline-start: 1ch;").is_empty());
        assert!(vertical_tokens("color: red;").is_empty());
    }

    #[test]
    fn vertical_violations_exempt_only_named_selectors() {
        let hidden = "\n.sr-only {\n  margin: -1px;\n}\n";
        assert!(vertical_violations(hidden).is_empty());
        let cell = "\nr-tree li {\n  padding: 2px 0;\n}\n";
        assert!(vertical_violations(cell).is_empty());
        let other = "\nmain {\n  padding: 2px 0;\n}\n";
        assert_eq!(vertical_violations(other), vec!["padding: 2px 0;"]);
    }

    #[test]
    fn vertical_spacing_uses_the_20px_grid() {
        let css = css();
        assert_eq!(vertical_violations(&css), Vec::<String>::new());
    }

    #[test]
    fn vertical_violations_flag_rem_and_off_grid_values() {
        for old in [
            "margin: 0.25rem 0;",
            "padding: 0.25rem 1ch;",
            "margin: 0.5rem 0;",
            "padding: 7px 1ch;",
        ] {
            let css = format!("a {{\n  {old}\n}}\n");
            assert_eq!(vertical_violations(&css), vec![old.to_string()], "{old}");
        }
        let fine =
            "a {\n  margin: 20px 0 10px;\n  padding: 0 2ch;\n  margin-inline-start: 1ch;\n}\n";
        assert!(vertical_violations(fine).is_empty());
    }

    #[test]
    fn table_cells_and_tree_rows_are_24px() {
        let css = css();
        assert_declares(&css, "r-table th,\nr-table td", "padding: 2px 1ch;");
        assert_declares(&css, "r-prose th,\nr-prose td", "padding: 2px 1ch;");
        assert_declares(&css, "r-tree li", "padding: 2px 0;");
        assert!(!rule_body(&css, "r-tree li").unwrap().contains("min-height"));
    }

    #[test]
    fn prose_code_and_grid_spacing_follow_the_design_values() {
        let css = css();
        let headings =
            "r-prose h1,\nr-prose h2,\nr-prose h3,\nr-prose h4,\nr-prose h5,\nr-prose h6";
        assert_declares(&css, headings, "margin: 20px 0 10px;");
        assert_declares(&css, "r-prose hr", "margin: 20px 0;");
        assert_declares(&css, "r-code", "margin: 20px 0 10px;");
        assert_declares(&css, "[data-theme]", "padding: 20px 2ch 30px;");
        assert_declares(&css, ".columns", "gap: 20px 2ch;");
        assert_declares(&css, ".bar,\n.nav", "min-height: 30px;");
        assert_declares(&css, ".bar,\n.nav", "align-items: center;");
    }

    #[test]
    fn the_kv_key_keeps_the_base_size() {
        let css = css();
        let body = rule_body(&css, "r-kv dt").unwrap();
        assert!(!body.contains("font-size"), "{body}");
    }

    #[test]
    fn catalogue_state_names_are_12px_and_group_names_keep_the_base() {
        let css = css();
        assert_declares(&css, ".state h4", "font-size: 12px;");
        assert_declares(&css, ".state h4", "font-weight: 400;");
        assert_declares(&css, ".state h4", "color: var(--r-muted);");
        assert_declares(&css, ".group h3", "font-size: 1em;");
    }

    #[test]
    fn the_catalogue_theme_heading_is_on_the_scale() {
        let css = css();
        for declaration in ["margin: 0 0 10px;", "font-size: 1em;", "font-weight: 600;"] {
            assert_declares(&css, "[data-theme] > h2", declaration);
        }
    }

    #[test]
    fn the_pill_line_is_16px() {
        assert_declares(&css(), "r-pill", "line-height: 16px;");
    }

    #[test]
    fn code_uses_the_page_font() {
        let css = css();
        for selector in ["r-code pre", "r-prose code"] {
            assert_declares(&css, selector, "font: inherit;");
        }
    }

    #[test]
    fn controls_use_the_page_font() {
        let css = css();
        assert_declares(&css, "button,\nselect", "font: inherit;");
    }

    #[test]
    fn paragraphs_sit_on_the_10px_grid_everywhere() {
        let css = css();
        assert_declares(&css, "p", "margin: 10px 0;");
        assert!(rule_body(&css, "r-prose p").is_none());
    }
}
