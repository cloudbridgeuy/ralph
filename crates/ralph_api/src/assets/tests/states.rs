use super::*;

#[test]
fn btn_is_a_bordered_twenty_pixel_control() {
    let css = css();
    for declaration in [
        "height: 20px;",
        "padding: 0 2ch;",
        "border: 1px solid var(--r-line-strong);",
        "border-radius: 3px;",
        "background: var(--r-bg);",
        "color: var(--r-ink);",
        "font: inherit;",
        "cursor: pointer;",
    ] {
        assert_declares(&css, ".btn", declaration);
    }
}

#[test]
fn btn_hover_and_disabled_use_existing_roles() {
    let css = css();
    assert_declares(&css, ".btn:hover", "background: var(--r-surface);");
    assert_declares(&css, ".btn:disabled", "color: var(--r-muted);");
    assert_declares(&css, ".btn:disabled", "border-color: var(--r-line);");
}

#[test]
fn btn_never_uses_the_faint_role() {
    let css = css();
    for selector in [".btn", ".btn:hover", ".btn:disabled"] {
        let body = rule_body(&css, selector).unwrap();
        assert!(!body.contains("--r-faint"), "{selector}");
    }
}

#[test]
fn empty_is_a_muted_block_with_ink_what_and_muted_next() {
    let css = css();
    assert_declares(&css, "r-empty", "color: var(--r-muted);");
    assert_declares(&css, "r-empty", "padding: 10px 0;");
    assert_declares(&css, "r-empty .what", "color: var(--r-ink);");
    assert_declares(&css, "r-empty .next", "color: var(--r-muted);");
    assert_declares(&css, "r-empty .next", "margin-top: 10px;");
}

#[test]
fn empty_compact_drops_its_padding() {
    assert_declares(&css(), "r-empty[compact]", "padding: 0;");
}

#[test]
fn error_keeps_the_bad_bar_on_a_surface_box() {
    let css = css();
    for declaration in [
        "padding: 10px 2ch;",
        "border: 1px solid var(--r-line-strong);",
        "border-left: 2px solid var(--r-bad);",
        "background: var(--r-surface);",
    ] {
        assert_declares(&css, "r-error", declaration);
    }
    let body = rule_body(&css, "r-error").unwrap();
    assert!(body.find("border:").unwrap() < body.find("border-left:").unwrap());
}

#[test]
fn error_lines_use_bad_ink_and_muted() {
    let css = css();
    assert_declares(&css, "r-error .what", "color: var(--r-bad);");
    assert_declares(&css, "r-error .what", "font-weight: 600;");
    assert_declares(&css, "r-error .why", "color: var(--r-ink);");
    assert_declares(&css, "r-error .next", "color: var(--r-muted);");
    assert_declares(&css, "r-error .next", "margin-top: 10px;");
}

#[test]
fn error_compact_is_one_flex_row_with_next_on_the_right() {
    let css = css();
    for declaration in ["display: flex;", "gap: 2ch;", "padding: 0 2ch;"] {
        assert_declares(&css, "r-error[compact]", declaration);
    }
    assert_declares(&css, "r-error[compact] .next", "margin-left: auto;");
}

#[test]
fn state_rules_add_no_tint_or_literal_colour() {
    let css = css();
    for selector in [
        "r-empty",
        "r-empty .what",
        "r-empty .next",
        "r-error",
        "r-error .what",
        "r-error .why",
        "r-error .next",
    ] {
        let body = rule_body(&css, selector).unwrap();
        assert!(
            !body.contains("color-mix") && !body.contains('#'),
            "{selector}"
        );
    }
}
