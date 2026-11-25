use super::{assert_declares, css, rule_body};

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

#[test]
fn a_state_row_pads_its_cell_on_the_grid_and_has_no_hover_fill() {
    let css = css();
    assert_declares(&css, "r-table tbody tr.state > td", "padding: 10px 1ch;");
    assert_declares(
        &css,
        "r-table tbody tr.state:hover > td",
        "background: none;",
    );
}

fn narrow_block(css: &str) -> &str {
    let start = css.find("@media (max-width: 760px) {").unwrap();
    let rest = &css[start..];
    let end = rest.find("\n}\n").unwrap();
    &rest[..end]
}

fn narrow_rule<'a>(block: &'a str, selector: &str) -> &'a str {
    let head = format!("\n  {selector} {{\n");
    let start = block
        .find(&head)
        .unwrap_or_else(|| panic!("{selector} has no rule"))
        + head.len();
    let end = block[start..].find('}').unwrap();
    &block[start..start + end]
}

#[test]
fn the_stackable_table_stacks_at_760px_or_narrower() {
    let css = css();
    let block = narrow_block(&css);
    for needle in [
        "r-table.stackable thead {\n    display: none;",
        "r-table.stackable tbody tr:not(.state) {\n    display: grid;",
        "r-table.stackable td.clip {\n    grid-column: 1 / -1;",
    ] {
        assert!(block.contains(needle), "{needle}");
    }
}

#[test]
fn the_stacked_text_line_wraps_to_two_lines_never_per_character() {
    let css = css();
    let block = narrow_block(&css);
    assert!(block.contains("-webkit-line-clamp: 2;"));
    assert!(block.contains("white-space: normal;"));
    assert!(!block.contains("overflow-wrap: anywhere"));
    assert!(!block.contains("word-break: break-all"));
}

#[test]
fn stacked_rows_keep_the_states_and_pad_on_the_grid() {
    let css = css();
    let block = narrow_block(&css);
    assert!(block.contains("padding: 10px 1ch 10px 0;"));
    assert!(block.contains("r-table.stackable tbody tr.open"));
    assert!(block.contains("r-table.stackable tbody tr[aria-selected=\"true\"]"));
    assert!(!block.contains("5px"));
    let hover = narrow_rule(block, "r-table.stackable tbody tr:hover");
    assert!(hover.contains("background: var(--r-hover);"), "{hover}");
    let selected = narrow_rule(block, "r-table.stackable tbody tr[aria-selected=\"true\"]");
    assert!(
        selected.contains("background: var(--r-selected);"),
        "{selected}"
    );
    assert!(
        selected.contains("border-left-color: var(--r-accent);"),
        "{selected}"
    );
    let open = narrow_rule(block, "r-table.stackable tbody tr.open");
    assert!(
        open.contains("border-left-color: var(--r-accent);"),
        "{open}"
    );
}

#[test]
fn only_the_stackable_class_changes_a_table_at_760px() {
    let css = css();
    let block = narrow_block(&css);
    for line in block.lines().filter(|line| line.contains("r-table")) {
        assert!(line.contains("r-table.stackable"), "{line}");
    }
}
