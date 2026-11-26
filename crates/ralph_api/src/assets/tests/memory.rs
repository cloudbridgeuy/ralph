use super::*;
use std::collections::BTreeSet;

const APP_CSS_CLASSES: [&str; 19] = [
    "g",
    "chev",
    "id",
    "n",
    "extent",
    "track",
    "span",
    "sum",
    "stackable",
    "num",
    "clip",
    "state",
    "aside",
    "title",
    "what",
    "why",
    "next",
    "btn",
    "icon",
];

fn memory_css() -> String {
    format!("\n{}", include_str!("../../../assets/css/memory.css"))
}

fn threads_css() -> String {
    format!("\n{}", include_str!("../../../assets/css/threads.css"))
}

fn script_classes(js: &str) -> BTreeSet<String> {
    js.split("class: \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .flat_map(|names| names.split(' ').map(str::to_string))
        .collect()
}

fn selector_classes(css: &str) -> BTreeSet<String> {
    css.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('@') && (line.ends_with('{') || line.ends_with(',')))
        .flat_map(|line| {
            line.split('.').skip(1).map(|rest| {
                rest.chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .collect::<String>()
            })
        })
        .filter(|name| !name.is_empty())
        .collect()
}

#[test]
fn script_classes_read_every_class_attribute_value() {
    let js = "el(\"a\", null, { class: \"x y\" }); el(\"b\", null, { class: \"z\" });";
    let found: Vec<String> = script_classes(js).into_iter().collect();
    assert_eq!(found, vec!["x", "y", "z"]);
}

#[test]
fn selector_classes_read_selector_lines_only() {
    let css = "\n.a .b,\n.c:hover {\n  margin: 0.5px;\n}\n\n@media (max-width: 760px) {\n  .d {\n    color: red;\n  }\n}\n";
    let found: Vec<String> = selector_classes(css).into_iter().collect();
    assert_eq!(found, vec!["a", "b", "c", "d"]);
}

#[test]
fn memory_css_is_in_the_app_sheet_before_threads_css_which_is_last() {
    assert!(find("/assets/memory.css").is_none());
    assert!(find("/assets/threads.css").is_none());
    assert!(!shell().contains("memory.css"));
    assert!(!shell().contains("threads.css"));
    let app = css();
    let memory = memory_css();
    let threads = threads_css();
    let at = app.find(memory.trim_start()).unwrap();
    assert!(at > app.find("r-tree button:focus-visible").unwrap());
    assert!(app.find(threads.trim_start()).unwrap() > at);
    assert!(app.ends_with(threads.trim_start()));
}

#[test]
fn css_files_stay_under_1000_lines() {
    for (name, text) in [
        ("css/base.css", include_str!("../../../assets/css/base.css")),
        (
            "css/table.css",
            include_str!("../../../assets/css/table.css"),
        ),
        ("css/data.css", include_str!("../../../assets/css/data.css")),
        (
            "css/states.css",
            include_str!("../../../assets/css/states.css"),
        ),
        (
            "css/controls.css",
            include_str!("../../../assets/css/controls.css"),
        ),
        ("css/code.css", include_str!("../../../assets/css/code.css")),
        ("css/tree.css", include_str!("../../../assets/css/tree.css")),
        (
            "css/responsive.css",
            include_str!("../../../assets/css/responsive.css"),
        ),
        (
            "css/memory.css",
            include_str!("../../../assets/css/memory.css"),
        ),
        (
            "css/threads.css",
            include_str!("../../../assets/css/threads.css"),
        ),
    ] {
        let lines = text.lines().count();
        assert!(lines < 1000, "{name} has {lines} lines");
    }
}

#[test]
fn split_is_two_columns_and_stacks_at_760() {
    let css = memory_css();
    assert_declares(&css, ".split", "display: grid;");
    assert_declares(
        &css,
        ".split",
        "grid-template-columns: minmax(58ch, 40%) 1fr;",
    );
    let media = css.find("@media (max-width: 760px)").unwrap();
    let stacked = &css[media..];
    assert!(stacked.contains(".split {"));
    assert!(stacked.contains("grid-template-columns: 1fr;"));
}

#[test]
fn focus_visible_covers_rows_tabs_and_crumbs() {
    let css = memory_css();
    for selector in [
        ".tabs a:focus-visible",
        ".crumbs a:focus-visible",
        "r-tree li > button:focus-visible",
    ] {
        assert!(css.contains(selector), "{selector}");
    }
    assert!(css.contains("outline: 2px solid var(--r-accent);"));
    assert!(css.contains("outline-offset: -2px;"));
    assert!(css.contains("outline-offset: 1px;"));
}

#[test]
fn tabs_mark_the_current_thread_with_the_accent_bar() {
    let css = memory_css();
    let current = ".tabs a[aria-current=\"page\"]";
    assert_declares(&css, current, "border-bottom-color: var(--r-accent);");
    assert_declares(&css, current, "color: var(--r-ink-strong);");
    assert_declares(&css, ".tabs a", "border-bottom: 2px solid transparent;");
    assert_declares(&css, ".tabs", "overflow-x: auto;");
    assert_declares(&css, ".tabs", "white-space: nowrap;");
}

const ROW_BAR: &str = "box-shadow: inset 2px 0 0 var(--r-accent);";
const ROW_SELECTED: &str = "r-tree li[aria-selected=\"true\"]::before";
const ROW_HOVER: &str = "r-tree li[role=\"treeitem\"]:not([aria-selected=\"true\"]):hover:not(:has(> ul[role=\"group\"]:hover))::before";
const ROW_BASE: &str = "r-tree li[role=\"treeitem\"]::before";

#[test]
fn the_selected_tree_row_has_the_selected_fill_and_accent_bar() {
    let css = memory_css();
    assert_declares(&css, ROW_SELECTED, "background: var(--r-selected);");
    assert_declares(&css, ROW_SELECTED, ROW_BAR);
}

#[test]
fn a_hovered_tree_row_has_the_hover_fill_and_the_same_accent_bar() {
    let css = memory_css();
    assert_declares(&css, ROW_HOVER, "background: var(--r-hover);");
    assert_declares(&css, ROW_HOVER, ROW_BAR);
    assert!(!rule_body(&css, ROW_HOVER).unwrap().contains("--r-selected"));
}

#[test]
fn the_row_fill_cannot_cover_the_child_group_or_collapse() {
    let css = memory_css();
    assert_declares(&css, ROW_BASE, "content: \"\";");
    assert_declares(&css, ROW_BASE, "grid-column: 1 / -1;");
    assert_declares(&css, ROW_BASE, "grid-row: 1;");
    assert_declares(&css, ROW_BASE, "align-self: stretch;");
    assert!(ROW_HOVER.contains(":not(:has(> ul[role=\"group\"]:hover))"));
    assert!(ROW_HOVER.contains(":not([aria-selected=\"true\"])"));
}

#[test]
fn the_selected_selector_matches_the_attribute_the_script_sets_on_the_row() {
    let js = script();
    assert!(ROW_SELECTED.contains("li[aria-selected=\"true\"]"));
    assert!(js.contains("item.setAttribute(\"aria-selected\", \"true\");"));
    assert!(js.contains("function markSelected(list, item)"));
    assert!(js.contains("markSelected(tree, item);"));
    assert!(js.contains("markSelected(list, item);"));
    assert!(js.contains("const item = button.parentElement;"));
    assert!(js.contains("el(\"li\", null, { role: \"treeitem\""));
}

#[test]
fn every_script_class_has_a_rule_and_every_memory_rule_is_used() {
    let app = css();
    for owned in APP_CSS_CLASSES {
        assert!(app.contains(&format!(".{owned}")), "app.css lacks .{owned}");
    }
    let used = script_classes(&script());
    let ruled: BTreeSet<String> = selector_classes(&memory_css())
        .union(&selector_classes(&threads_css()))
        .cloned()
        .collect();
    for name in used
        .iter()
        .filter(|name| !APP_CSS_CLASSES.contains(&name.as_str()))
    {
        assert!(
            ruled.contains(name),
            "css/memory.css and css/threads.css lack a rule for .{name}"
        );
    }
    for name in &ruled {
        assert!(used.contains(name), "the script never sets .{name}");
    }
    assert!(!ruled.is_empty());
}

#[test]
fn the_bundled_memory_css_uses_only_role_tokens() {
    let css = memory_css();
    assert!(!css.contains('#'));
    assert!(!css.contains("rgb"));
    assert!(!css.contains("hsl"));
    assert!(css.contains("var(--r-"));
}

#[test]
fn the_pager_buttons_take_their_look_from_controls_and_memory_adds_layout_and_disabled() {
    let css = memory_css();
    assert!(!css.contains("\n.btn "));
    assert!(!css.contains("\n.btn:"));
    let controls = format!("\n{}", include_str!("../../../assets/css/controls.css"));
    assert!(controls.contains("\n.btn {\n"));
    assert_declares(&css, ".pager .btn", "display: inline-flex;");
    assert_declares(&css, ".pager .btn", "align-items: center;");
    assert_declares(
        &css,
        ".pager .btn[aria-disabled=\"true\"]",
        "color: var(--r-muted);",
    );
    assert_declares(
        &css,
        ".pager .btn[aria-disabled=\"true\"]",
        "border-color: var(--r-line);",
    );
}

#[test]
fn a_disabled_pager_link_keeps_its_look_on_hover() {
    let css = memory_css();
    let rule = ".pager .btn[aria-disabled=\"true\"]:hover";
    assert_declares(&css, rule, "background: var(--r-bg);");
    assert_declares(&css, rule, "color: var(--r-muted);");
    assert_declares(&css, rule, "border-color: var(--r-line);");
    assert_declares(&css, rule, "cursor: default;");
}

#[test]
fn kbd_uses_role_tokens() {
    let css = memory_css();
    assert_declares(&css, "kbd", "font-size: 12px;");
    assert_declares(&css, "kbd", "border: 1px solid var(--r-line-strong);");
    assert_declares(&css, "kbd", "border-radius: 3px;");
    assert_declares(&css, "kbd", "color: var(--r-muted);");
}

#[test]
fn entry_rows_have_a_focus_visible_outline() {
    let css = memory_css();
    let rule = "r-table tbody tr[tabindex]:focus-visible";
    assert_declares(&css, rule, "outline: 2px solid var(--r-accent);");
    assert_declares(&css, rule, "outline-offset: -2px;");
    assert_declares(
        &css,
        "r-table tbody tr.full > td:first-child",
        "border-left-color: var(--r-accent);",
    );
}

#[test]
fn the_no_summary_text_is_muted_plain_text() {
    let css = memory_css();
    assert_declares(&css, ".none", "color: var(--r-muted);");
    assert!(!rule_body(&css, ".none").unwrap().contains("border"));
}

#[test]
fn the_entries_table_keeps_every_cell_but_the_text_on_one_line() {
    let css = memory_css();
    let cells = "r-table.stackable td:not(.clip)";
    assert_declares(&css, cells, "white-space: nowrap;");
    assert_declares(&css, cells, "overflow-wrap: normal;");
}

#[test]
fn each_role_attribute_maps_to_its_role_token() {
    let css = memory_css();
    assert_declares(&css, "[data-role=\"assistant\"]", "color: var(--r-accent);");
    assert_declares(&css, "[data-role=\"user\"]", "color: var(--r-ink);");
    assert_declares(&css, "[data-role=\"worker\"]", "color: var(--r-info);");
    assert_eq!(css.matches("data-role=").count(), 3);
}

#[test]
fn times_and_the_extent_bar_are_muted_and_range_labels_stay_bold() {
    let css = memory_css();
    assert_declares(&css, ".at", "color: var(--r-muted);");
    assert_declares(&css, "r-tree .extent .span", "fill: var(--r-faint);");
    assert_declares(&css, "r-tree .sum", "color: var(--r-ink);");
    let app = super::css();
    assert_declares(&app, "r-tree .n", "color: var(--r-muted);");
    assert_declares(&app, "r-tree .extent .track", "fill: var(--r-line);");
    assert_declares(&app, "r-tree li > button", "font-weight: 600;");
    assert_declares(&app, "r-tree .id", "color: var(--r-ink-strong);");
}

#[test]
fn every_colour_in_memory_css_is_a_role_token() {
    let css = memory_css();
    let colours: Vec<&str> = css
        .lines()
        .map(str::trim)
        .filter(|line| {
            ["color:", "background:", "fill:", "stroke:"]
                .iter()
                .any(|property| line.starts_with(property))
        })
        .collect();
    assert!(!colours.is_empty());
    for line in colours {
        assert!(
            line.contains("var(--r-") || line.ends_with("none;") || line.ends_with("inherit;"),
            "{line}"
        );
    }
}
