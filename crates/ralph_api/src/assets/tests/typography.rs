use super::*;

#[test]
fn the_body_is_13px_on_a_20px_line() {
    let css = css();
    assert_declares(&css, "body", "font-size: 13px;");
    assert_declares(&css, "body", "line-height: 20px;");
}

#[test]
fn meta_text_is_12px() {
    let css = css();
    for selector in [
        ".note",
        ".badge",
        ".store",
        "r-table th",
        "r-prose th",
        "r-pill",
        "r-spark",
    ] {
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
        "r-box > .title",
        "r-status .mark",
        "r-prose strong",
        "r-tree li > button",
        "r-prose h1,\nr-prose h2,\nr-prose h3,\nr-prose h4,\nr-prose h5,\nr-prose h6",
    ] {
        assert_declares(&css, selector, "font-weight: 600;");
    }
}
