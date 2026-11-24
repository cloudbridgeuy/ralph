use super::*;

#[test]
fn box_is_positioned_so_the_title_can_sit_in_the_border() {
    let css = css();
    assert_declares(&css, "r-box", "position: relative;");
    assert_declares(&css, "r-box", "margin: 10px 0;");
}

#[test]
fn box_rule_has_no_overflow_except_wrap() {
    let css = css();
    let body = rule_body(&css, "r-box").unwrap();
    for line in declarations(body, "overflow") {
        assert!(line.starts_with("overflow-wrap:"), "{line}");
    }
}

#[test]
fn box_title_sits_in_the_top_border() {
    let css = css();
    for declaration in [
        "position: absolute;",
        "top: -10px;",
        "left: 1ch;",
        "line-height: 20px;",
        "padding: 0 1ch;",
        "background: var(--r-bg);",
        "white-space: nowrap;",
        "overflow: hidden;",
        "text-overflow: ellipsis;",
        "max-width: calc(60% - 2ch);",
        "font-weight: 600;",
        "color: var(--r-ink-strong);",
    ] {
        assert_declares(&css, "r-box > .title", declaration);
    }
}

#[test]
fn box_aside_sits_in_the_top_border_on_the_right() {
    let css = css();
    for declaration in [
        "position: absolute;",
        "top: -10px;",
        "right: 1ch;",
        "line-height: 20px;",
        "padding: 0 1ch;",
        "background: var(--r-bg);",
        "white-space: nowrap;",
        "overflow: hidden;",
        "text-overflow: ellipsis;",
        "max-width: calc(40% - 2ch);",
        "font-size: 12px;",
        "color: var(--r-muted);",
    ] {
        assert_declares(&css, "r-box > .aside", declaration);
    }
}

#[test]
fn box_aside_text_is_not_faint() {
    let css = css();
    let body = rule_body(&css, "r-box > .aside").unwrap();
    assert!(!body.contains("--r-faint"));
}

#[test]
fn heavy_box_has_a_2px_ink_border_and_no_radius() {
    let css = css();
    assert_declares(
        &css,
        "r-box[shape=\"heavy\"]",
        "border: 2px solid var(--r-ink);",
    );
    assert_declares(&css, "r-box[shape=\"heavy\"]", "border-radius: 0;");
}

#[test]
fn round_box_has_a_3px_radius() {
    let css = css();
    assert_declares(&css, "r-box[shape=\"round\"]", "border-radius: 3px;");
    assert!(!css.contains("0.75rem"));
}

#[test]
fn box_title_and_aside_reserve_their_own_offsets_so_they_never_overlap() {
    let css = css();
    assert_declares(&css, "r-box > .title", "left: 1ch;");
    assert_declares(&css, "r-box > .aside", "right: 1ch;");
    assert_declares(&css, "r-box > .title", "max-width: calc(60% - 2ch);");
    assert_declares(&css, "r-box > .aside", "max-width: calc(40% - 2ch);");
}
