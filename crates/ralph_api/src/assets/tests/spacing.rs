use super::*;

fn spacing_declarations(css: &str) -> Vec<&str> {
    let mut all = declarations(css, "margin");
    all.extend(declarations(css, "padding"));
    all
}

#[test]
fn spacing_declarations_collect_margins_and_paddings() {
    let css = "a {\n  margin: 0;\n  padding-left: 1ch;\n  color: red;\n}\n";
    assert_eq!(
        spacing_declarations(css),
        vec!["margin: 0;", "padding-left: 1ch;"]
    );
}

#[test]
fn page_gutters_are_2ch() {
    let css = css();
    assert_declares(&css, "main", "padding: 20px 2ch 30px;");
    assert_declares(&css, ".bar,\n.nav", "padding: 0 2ch;");
    assert_declares(&css, "r-box", "padding: 10px 2ch;");
    assert_declares(&css, "r-prose", "max-width: 92ch;");
}

#[test]
fn spacing_has_no_rem() {
    let css = css();
    let spacing = spacing_declarations(&css);
    assert!(!spacing.is_empty());
    for line in spacing {
        assert!(!line.contains("rem"), "{line}");
    }
}

const SPACING_GRID: [&str; 4] = ["0", "10px", "20px", "30px"];
const FINE_PADDING_SELECTORS: [&str; 2] = ["r-table th,\nr-table td", "r-prose th,\nr-prose td"];

fn vertical_tokens(line: &str) -> Vec<&str> {
    let Some((property, value)) = line.trim_end_matches(';').split_once(": ") else {
        return Vec::new();
    };
    let values: Vec<&str> = value.split(' ').collect();
    match property {
        "margin" | "padding" => match values.len() {
            1 | 2 => vec![values[0]],
            _ => vec![values[0], values[2]],
        },
        "margin-top"
        | "margin-bottom"
        | "margin-block"
        | "margin-block-start"
        | "margin-block-end"
        | "padding-top"
        | "padding-bottom"
        | "padding-block"
        | "padding-block-start"
        | "padding-block-end" => values,
        _ => Vec::new(),
    }
}

fn vertical_violations(css: &str) -> Vec<String> {
    let mut found = Vec::new();
    for chunk in css.split('}') {
        let Some((head, body)) = chunk.rsplit_once('{') else {
            continue;
        };
        let selector = head.trim();
        for line in body.lines().map(str::trim) {
            let fine = line.starts_with("padding") && FINE_PADDING_SELECTORS.contains(&selector);
            let bad = vertical_tokens(line)
                .into_iter()
                .any(|token| !(SPACING_GRID.contains(&token) || fine && token == "2px"));
            if bad {
                found.push(line.to_string());
            }
        }
    }
    found
}

#[test]
fn vertical_tokens_pick_only_the_vertical_positions() {
    assert_eq!(
        vertical_tokens("margin: 1px 2px 3px 4px;"),
        vec!["1px", "3px"]
    );
    assert_eq!(vertical_tokens("padding: 1px 2px 3px;"), vec!["1px", "3px"]);
    assert_eq!(vertical_tokens("padding: 1px 2px;"), vec!["1px"]);
    assert_eq!(vertical_tokens("margin: 5px;"), vec!["5px"]);
    assert_eq!(
        vertical_tokens("margin-block: 1px 2px;"),
        vec!["1px", "2px"]
    );
    assert!(vertical_tokens("margin-inline-start: 1ch;").is_empty());
    assert!(vertical_tokens("color: red;").is_empty());
}

#[test]
fn vertical_violations_exempt_only_named_selectors() {
    let cell = "\nr-prose th,\nr-prose td {\n  padding: 2px 0;\n}\n";
    assert!(vertical_violations(cell).is_empty());
    let other = "\nmain {\n  padding: 2px 0;\n}\n";
    assert_eq!(vertical_violations(other), vec!["padding: 2px 0;"]);
}

#[test]
fn vertical_spacing_uses_the_20px_grid() {
    let css = css();
    assert_eq!(vertical_violations(&css), Vec::<String>::new());
}

#[test]
fn vertical_violations_flag_rem_and_off_grid_values() {
    for old in [
        "margin: 0.25rem 0;",
        "padding: 0.25rem 1ch;",
        "margin: 0.5rem 0;",
        "padding: 7px 1ch;",
    ] {
        let css = format!("a {{\n  {old}\n}}\n");
        assert_eq!(vertical_violations(&css), vec![old.to_string()], "{old}");
    }
    let fine = "a {\n  margin: 20px 0 10px;\n  padding: 0 2ch;\n  margin-inline-start: 1ch;\n}\n";
    assert!(vertical_violations(fine).is_empty());
}

#[test]
fn table_cells_and_tree_rows_are_24px() {
    let css = css();
    assert_declares(&css, "r-table th,\nr-table td", "padding: 2px 1ch;");
    assert_declares(&css, "r-prose th,\nr-prose td", "padding: 2px 1ch;");
    assert_declares(&css, "r-tree li > button,\nr-tree .label", "height: 24px;");
    let row = rule_body(&css, "r-tree li").unwrap();
    assert!(
        row.contains("padding: 0;") && !row.contains("min-height"),
        "{row}"
    );
}

#[test]
fn prose_code_and_grid_spacing_follow_the_design_values() {
    let css = css();
    let headings = "r-prose h1,\nr-prose h2,\nr-prose h3,\nr-prose h4,\nr-prose h5,\nr-prose h6";
    assert_declares(&css, headings, "margin: 20px 0 10px;");
    assert_declares(&css, "r-prose hr", "margin: 20px 0;");
    assert_declares(&css, "r-code", "margin: 20px 0 10px;");
    assert_declares(&css, "[data-theme]", "padding: 20px 2ch 30px;");
    assert_declares(&css, ".columns", "gap: 20px 2ch;");
    assert_declares(&css, ".bar,\n.nav", "min-height: 30px;");
    assert_declares(&css, ".bar,\n.nav", "align-items: center;");
}

#[test]
fn the_kv_key_keeps_the_base_size() {
    let css = css();
    let body = rule_body(&css, "r-kv dt").unwrap();
    assert!(!body.contains("font-size"), "{body}");
}

#[test]
fn the_kv_has_three_columns_key_value_source() {
    let css = css();
    assert_declares(
        &css,
        "r-kv dl",
        "grid-template-columns: max-content minmax(0, 1fr) max-content;",
    );
    assert_declares(&css, "r-kv dt", "color: var(--r-muted);");
    assert_declares(&css, "r-kv dd", "color: var(--r-ink-strong);");
}

#[test]
fn the_kv_source_is_12px_right_aligned_muted_and_medium_weight_when_set() {
    let css = css();
    assert_declares(&css, "r-kv dd.src", "font-size: 12px;");
    assert_declares(&css, "r-kv dd.src", "text-align: right;");
    assert_declares(&css, "r-kv dd.src", "color: var(--r-muted);");
    assert_declares(&css, "r-kv dd.src.set", "color: var(--r-muted);");
    assert_declares(&css, "r-kv dd.src.set", "font-weight: 500;");
    assert!(!rule_body(&css, "r-kv dd.src")
        .unwrap()
        .contains("--r-faint"));
}

#[test]
fn the_kv_two_variant_drops_the_source_column() {
    let css = css();
    assert_declares(
        &css,
        "r-kv[two] dl",
        "grid-template-columns: max-content minmax(0, 1fr);",
    );
}

#[test]
fn the_kv_inline_variant_is_one_wrapping_stat_line() {
    let css = css();
    assert_declares(&css, "r-kv[inline] dl", "display: flex;");
    assert_declares(&css, "r-kv[inline] dl", "flex-wrap: wrap;");
}

#[test]
fn catalogue_state_names_are_12px_and_group_names_keep_the_base() {
    let css = css();
    assert_declares(&css, ".state h4", "font-size: 12px;");
    assert_declares(&css, ".state h4", "font-weight: 400;");
    assert_declares(&css, ".state h4", "color: var(--r-muted);");
    assert_declares(&css, ".group h3", "font-size: 1em;");
}

#[test]
fn the_catalogue_theme_heading_is_on_the_scale() {
    let css = css();
    for declaration in ["margin: 0 0 10px;", "font-size: 1em;", "font-weight: 600;"] {
        assert_declares(&css, "[data-theme] > h2", declaration);
    }
}

#[test]
fn the_pill_line_is_16px() {
    assert_declares(&css(), "r-pill", "line-height: 16px;");
}

#[test]
fn code_uses_the_page_font() {
    let css = css();
    for selector in ["r-code pre", "r-prose code"] {
        assert_declares(&css, selector, "font: inherit;");
    }
}

#[test]
fn controls_use_the_page_font() {
    let css = css();
    assert_declares(&css, "button,\nselect", "font: inherit;");
}

#[test]
fn paragraphs_sit_on_the_10px_grid_everywhere() {
    let css = css();
    assert_declares(&css, "p", "margin: 10px 0;");
    assert!(rule_body(&css, "r-prose p").is_none());
}
