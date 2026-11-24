use super::tests::{assert_declares, css, rule_body};

#[test]
fn the_table_header_is_muted_12px_and_does_not_wrap() {
    let css = css();
    assert_declares(&css, "r-table th", "font-size: 12px;");
    assert_declares(&css, "r-table th", "color: var(--r-muted);");
    assert_declares(&css, "r-table th", "white-space: nowrap;");
}

#[test]
fn numeric_cells_align_right_with_tabular_digits() {
    let css = css();
    assert_declares(&css, "r-table .num", "text-align: right;");
    assert_declares(&css, "r-table .num", "font-variant-numeric: tabular-nums;");
}

#[test]
fn a_clip_cell_cuts_with_an_ellipsis_on_the_real_width() {
    let css = css();
    for declaration in [
        "max-width: 0;",
        "width: 100%;",
        "white-space: nowrap;",
        "overflow: hidden;",
        "text-overflow: ellipsis;",
    ] {
        assert_declares(&css, "r-table .clip", declaration);
    }
}

const HOVER: &str = "r-table tbody tr:hover > td";
const SELECTED: &str = "r-table tbody tr[aria-selected=\"true\"] > td";
const BARS: &str = "r-table tbody tr[aria-selected=\"true\"] > td:first-child,\nr-table tbody tr.open > td:first-child";

#[test]
fn every_row_reserves_a_2px_left_bar_so_states_do_not_shift_cells() {
    let css = css();
    assert_declares(
        &css,
        "r-table th:first-child,\nr-table td:first-child",
        "border-left: 2px solid transparent;",
    );
}

#[test]
fn a_hovered_row_gets_the_hover_fill_and_no_text_colour_change() {
    let css = css();
    assert_declares(&css, HOVER, "background: var(--r-hover);");
    assert!(!rule_body(&css, HOVER).unwrap().contains("color:"));
}

#[test]
fn a_selected_row_gets_the_selected_fill_and_an_accent_bar() {
    let css = css();
    assert_declares(&css, SELECTED, "background: var(--r-selected);");
    assert_declares(&css, BARS, "border-left-color: var(--r-accent);");
}

#[test]
fn the_selected_fill_wins_over_the_hover_fill() {
    let css = css();
    let hover = css.find(&format!("\n{HOVER} {{")).unwrap();
    let selected = css.find(&format!("\n{SELECTED} {{")).unwrap();
    assert!(hover < selected);
}

#[test]
fn an_open_row_gets_an_accent_bar_and_no_fill() {
    let css = css();
    assert_declares(&css, BARS, "border-left-color: var(--r-accent);");
    assert!(rule_body(&css, "r-table tbody tr.open > td")
        .is_some_and(|body| !body.contains("background")));
}
