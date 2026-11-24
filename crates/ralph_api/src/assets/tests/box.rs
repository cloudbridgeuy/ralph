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
        "max-width: 60%;",
        "font-weight: 600;",
        "color: var(--r-ink-strong);",
    ] {
        assert_declares(&css, "r-box > .title", declaration);
    }
}
