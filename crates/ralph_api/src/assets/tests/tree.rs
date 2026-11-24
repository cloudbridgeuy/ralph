use super::*;

#[test]
fn tree_rows_are_a_four_column_grid() {
    let css = css();
    let row = "r-tree li";
    assert_declares(&css, row, "display: grid;");
    assert_declares(
        &css,
        row,
        "grid-template-columns: 24ch 7ch 9ch minmax(0, 1fr);",
    );
    assert_declares(&css, row, "column-gap: 1.5ch;");
    assert_declares(&css, "r-tree li[role=\"none\"]", "display: block;");
}

#[test]
fn tree_cells_stay_in_their_column_and_the_group_spans_the_row() {
    let css = css();
    let cells = [
        ("r-tree li > button,\nr-tree .label", "1"),
        ("r-tree .n", "2"),
        ("r-tree .extent", "3"),
        ("r-tree .sum", "4"),
    ];
    for (selector, column) in cells {
        assert_declares(&css, selector, &format!("grid-column: {column};"));
        assert_declares(&css, selector, "grid-row: 1;");
    }
    let group = "r-tree ul[role=\"group\"]";
    assert_declares(&css, group, "grid-column: 1 / -1;");
    assert_declares(&css, group, "grid-row: 2;");
    assert!(!rule_body(&css, group).unwrap().contains("border"));
}

#[test]
fn the_tree_summary_clips_on_one_line_and_the_count_aligns_right() {
    let css = css();
    for declaration in [
        "white-space: nowrap;",
        "overflow: hidden;",
        "text-overflow: ellipsis;",
        "color: var(--r-muted);",
    ] {
        assert_declares(&css, "r-tree .sum", declaration);
    }
    assert_declares(&css, "r-tree .n", "text-align: right;");
    assert_declares(&css, "r-tree .n", "font-variant-numeric: tabular-nums;");
}

#[test]
fn the_tree_chevron_is_a_role_token_stroke_that_turns_when_open() {
    let css = css();
    let chevron = "r-tree .chev";
    assert_declares(&css, chevron, "width: 8px;");
    assert_declares(&css, chevron, "height: 8px;");
    assert_declares(&css, chevron, "stroke: var(--r-muted);");
    let open = "r-tree li[aria-expanded=\"true\"] > button .chev";
    assert_declares(&css, open, "transform: rotate(90deg);");
    assert_declares(&css, open, "stroke: var(--r-ink-strong);");
}

#[test]
fn tree_guides_and_the_extent_bar_use_role_tokens() {
    let css = css();
    assert_declares(&css, "r-tree .g", "width: 2ch;");
    assert_declares(
        &css,
        "r-tree .g::before",
        "border-inline-start: 1px solid var(--r-line-strong);",
    );
    assert_declares(&css, "r-tree .extent", "height: 6px;");
    assert_declares(&css, "r-tree .extent .track", "fill: var(--r-line);");
    assert_declares(&css, "r-tree .extent .span", "fill: var(--r-muted);");
}

#[test]
fn the_old_tree_text_markers_are_gone() {
    let css = css();
    assert!(!css.contains("r-tree .meta"));
    assert!(!css.contains("content: \"- \""));
    assert!(!css.contains("content: \"+ \""));
}
