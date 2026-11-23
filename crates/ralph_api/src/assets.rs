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
        for tone in ["ok", "bad", "warn", "info", "accent"] {
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

    const DEFAULT_TOKENS: &str = include_str!("../../ralph/src/golden/tokens_default.css");

    fn theme_hex(theme: &str, name: &str) -> [f64; 3] {
        let block = DEFAULT_TOKENS
            .split(&format!("[data-theme=\"{theme}\"] {{"))
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .unwrap();
        let line = block
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{name}:")))
            .unwrap();
        let hex = line.split('#').nth(1).unwrap();
        let channel = |at: usize| f64::from(u8::from_str_radix(&hex[at..at + 2], 16).unwrap());
        [channel(0), channel(2), channel(4)]
    }

    fn luminance(rgb: [f64; 3]) -> f64 {
        let linear = |value: f64| {
            let unit = value / 255.0;
            if unit <= 0.04045 {
                unit / 12.92
            } else {
                ((unit + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])
    }

    fn contrast(a: [f64; 3], b: [f64; 3]) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    fn muted_weight() -> f64 {
        let css = css();
        let rest = css
            .split("color-mix(in srgb, var(--r-ansi-7) ")
            .nth(1)
            .unwrap();
        let percent = rest.split('%').next().unwrap();
        percent.parse::<f64>().unwrap() / 100.0
    }

    #[test]
    fn muted_text_has_aa_contrast_in_both_themes() {
        let weight = muted_weight();
        for theme in ["dark", "light"] {
            let fg = theme_hex(theme, "--r-ansi-7");
            let bg = theme_hex(theme, "--r-ansi-0");
            let mixed = [0, 1, 2].map(|i| (weight * fg[i] + (1.0 - weight) * bg[i]).round());
            let ratio = contrast(mixed, bg);
            assert!(ratio >= 4.5, "{theme}: {ratio:.2}");
        }
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
}
