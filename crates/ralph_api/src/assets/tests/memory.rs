use super::*;
use std::collections::BTreeSet;

const APP_CSS_CLASSES: [&str; 12] = [
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
];

fn memory_css() -> String {
    let text = String::from_utf8(bytes_of("/assets/memory.css").to_vec()).unwrap();
    format!("\n{text}")
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
fn memory_css_is_served_and_linked() {
    let sheet = find("/assets/memory.css").unwrap();
    assert_eq!(sheet.content_type, "text/css; charset=utf-8");
    assert!(!memory_css().trim().is_empty());
    let html = shell();
    let app = html
        .find("<link rel=\"stylesheet\" href=\"/assets/app.css\">")
        .unwrap();
    let memory = html
        .find("<link rel=\"stylesheet\" href=\"/assets/memory.css\">")
        .unwrap();
    assert!(app < memory);
}

#[test]
fn css_files_stay_under_1000_lines() {
    let sheets = ["/assets/app.css", "/assets/memory.css"];
    for path in sheets {
        let lines = String::from_utf8(bytes_of(path).to_vec())
            .unwrap()
            .lines()
            .count();
        assert!(lines < 1000, "{path} has {lines} lines");
    }
    assert!(memory_css().lines().count() > 1);
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

#[test]
fn the_selected_tree_row_has_the_selected_fill_and_accent_bar() {
    let css = memory_css();
    let selected = "r-tree li[aria-selected=\"true\"]::before";
    assert_declares(&css, selected, "background: var(--r-selected);");
    assert_declares(&css, selected, "var(--r-accent)");
}

#[test]
fn every_script_class_has_a_rule_and_every_memory_rule_is_used() {
    let app = css();
    for owned in APP_CSS_CLASSES {
        assert!(app.contains(&format!(".{owned}")), "app.css lacks .{owned}");
    }
    let used = script_classes(&script());
    let ruled = selector_classes(&memory_css());
    for name in used
        .iter()
        .filter(|name| !APP_CSS_CLASSES.contains(&name.as_str()))
    {
        assert!(ruled.contains(name), "memory.css lacks a rule for .{name}");
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
fn btn_and_kbd_use_role_tokens_and_a_focus_ring() {
    let css = memory_css();
    assert_declares(&css, ".btn", "border: 1px solid var(--r-line-strong);");
    assert_declares(&css, ".btn", "border-radius: 3px;");
    assert_declares(&css, ".btn", "padding: 0 1ch;");
    assert_declares(&css, ".btn:hover", "background: var(--r-hover);");
    assert_declares(
        &css,
        ".btn[aria-disabled=\"true\"]",
        "color: var(--r-muted);",
    );
    assert_declares(
        &css,
        ".btn:focus-visible",
        "outline: 2px solid var(--r-accent);",
    );
    assert_declares(&css, ".btn:focus-visible", "outline-offset: 1px;");
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
