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
