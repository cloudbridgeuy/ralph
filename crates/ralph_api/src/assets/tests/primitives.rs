use super::*;

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
fn spark_baseline_area_and_last_point_use_existing_roles() {
    let css = css();
    assert_declares(&css, "r-spark .base", "stroke: var(--r-line-strong);");
    assert_declares(&css, "r-spark .area", "fill: var(--r-hover);");
    assert!(!rule_body(&css, "r-spark .area")
        .unwrap()
        .contains("fill-opacity"));
    assert_declares(&css, "r-spark .area", "stroke: none;");
    assert_declares(&css, "r-spark .last", "fill: var(--r-accent);");
    assert_declares(&css, "r-spark .last", "stroke: none;");
}

#[test]
fn spark_last_value_is_bold_and_bars_use_muted_with_an_accent_highlight() {
    let css = css();
    assert_declares(&css, "r-spark b", "font-weight: 600;");
    assert_declares(&css, "r-spark b", "color: var(--r-ink-strong);");
    assert_declares(&css, "r-spark .bar", "fill: var(--r-muted);");
    assert_declares(&css, "r-spark .bar", "stroke: none;");
    assert_declares(&css, "r-spark .bar.hi", "fill: var(--r-accent);");
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
fn the_spinner_ring_has_no_css_delay_so_the_script_delay_is_the_only_one() {
    let css = css();
    let rule = rule_body(&css, "r-spinner::before").unwrap();
    assert!(
        rule.contains("animation: r-spin 800ms linear infinite;"),
        "{rule}"
    );
    assert!(!css.contains("200ms"));
    assert!(!css.contains("r-show"));
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
fn reduced_motion_mark_has_no_animation_and_no_delay() {
    let css = css();
    let media = css
        .split("@media (prefers-reduced-motion: reduce)")
        .nth(1)
        .unwrap();
    let rule = rule_body(media, "  r-spinner::before").unwrap();
    assert!(rule.contains("animation: none;"), "{rule}");
    assert!(!rule.contains("200ms"), "{rule}");
}

#[test]
fn reduced_motion_replaces_the_spinner_ring_with_a_static_mark() {
    let css = css();
    let media = css
        .split("@media (prefers-reduced-motion: reduce)")
        .nth(1)
        .unwrap();
    let rule = rule_body(media, "  r-spinner::before").unwrap();
    assert!(rule.contains("content: \""), "{rule}");
    assert!(!rule.contains("content: \"\""), "{rule}");
    assert!(rule.contains("border: 0"), "{rule}");
    assert!(rule.contains("var(--r-accent)"), "{rule}");
}

#[test]
fn the_spinner_ring_is_12px_in_the_accent_colour() {
    let css = css();
    let rule = rule_body(&css, "r-spinner::before").unwrap();
    assert!(rule.contains("width: 12px;"), "{rule}");
    assert!(
        rule.contains("border-top-color: var(--r-accent);"),
        "{rule}"
    );
}

#[test]
fn the_script_delays_the_spinner_200ms_and_passes_a_label() {
    let js = include_str!("../../../assets/app.js");
    assert!(js.contains("const SPINNER_DELAY_MS = 200;"));
    assert!(js.contains("function delayedSpinner(host, label)"));
    assert!(!js.contains("delayedSpinner(row)"));
    assert!(!js.contains("delayedSpinner(group)"));
    assert!(!js.contains("delayedSpinner(main)"));
    assert!(js.contains(
        "delayedSpinner(main, list ? \"Loading \" + list.name.toLowerCase() : config ? \"Loading config\" : \"Loading memory\")"
    ));
}

#[test]
fn the_stylesheet_has_the_two_theme_columns_and_no_unused_hidden_text_class() {
    let css = css();
    assert!(css.contains(".columns"));
    assert!(css.contains("[data-theme]"));
    assert!(!css.contains(".sr-only"));
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
    let sample = include_str!("../../../tests/golden/render/code_rust.html");
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

#[test]
fn the_stylesheet_has_no_data_uri() {
    assert!(!css().contains("data:"));
}
