use super::*;

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
