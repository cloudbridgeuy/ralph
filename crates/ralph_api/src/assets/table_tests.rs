use super::tests::{assert_declares, css};

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
