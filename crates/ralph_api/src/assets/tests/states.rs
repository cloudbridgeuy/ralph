use super::*;

#[test]
fn btn_is_a_bordered_twenty_pixel_control() {
    let css = css();
    for declaration in [
        "height: 20px;",
        "padding: 0 2ch;",
        "border: 1px solid var(--r-line-strong);",
        "border-radius: 3px;",
        "background: var(--r-bg);",
        "color: var(--r-ink);",
        "font: inherit;",
        "cursor: pointer;",
    ] {
        assert_declares(&css, ".btn", declaration);
    }
}

#[test]
fn btn_hover_and_disabled_use_existing_roles() {
    let css = css();
    assert_declares(&css, ".btn:hover", "background: var(--r-surface);");
    assert_declares(&css, ".btn:disabled", "color: var(--r-muted);");
    assert_declares(&css, ".btn:disabled", "border-color: var(--r-line);");
}

#[test]
fn btn_never_uses_the_faint_role() {
    let css = css();
    for selector in [".btn", ".btn:hover", ".btn:disabled"] {
        let body = rule_body(&css, selector).unwrap();
        assert!(!body.contains("--r-faint"), "{selector}");
    }
}

#[test]
fn empty_is_a_muted_block_with_ink_what_and_muted_next() {
    let css = css();
    assert_declares(&css, "r-empty", "color: var(--r-muted);");
    assert_declares(&css, "r-empty", "padding: 10px 0;");
    assert_declares(&css, "r-empty .what", "color: var(--r-ink);");
    assert_declares(&css, "r-empty .next", "color: var(--r-muted);");
    assert_declares(&css, "r-empty .next", "margin-top: 10px;");
}

#[test]
fn empty_compact_drops_its_padding() {
    assert_declares(&css(), "r-empty[compact]", "padding: 0;");
}

#[test]
fn error_keeps_the_bad_bar_on_a_surface_box() {
    let css = css();
    for declaration in [
        "padding: 10px 2ch;",
        "border: 1px solid var(--r-line-strong);",
        "border-left: 2px solid var(--r-bad);",
        "background: var(--r-surface);",
    ] {
        assert_declares(&css, "r-error", declaration);
    }
    let body = rule_body(&css, "r-error").unwrap();
    assert!(body.find("border:").unwrap() < body.find("border-left:").unwrap());
}

#[test]
fn error_lines_use_bad_ink_and_muted() {
    let css = css();
    assert_declares(&css, "r-error .what", "color: var(--r-bad);");
    assert_declares(&css, "r-error .what", "font-weight: 600;");
    assert_declares(&css, "r-error .why", "color: var(--r-ink);");
    assert_declares(&css, "r-error .next", "color: var(--r-muted);");
    assert_declares(&css, "r-error .next", "margin-top: 10px;");
}

#[test]
fn error_compact_is_one_flex_row_with_next_on_the_right() {
    let css = css();
    for declaration in ["display: flex;", "gap: 2ch;", "padding: 0 2ch;"] {
        assert_declares(&css, "r-error[compact]", declaration);
    }
    assert_declares(&css, "r-error[compact] .next", "margin-left: auto;");
}

#[test]
fn state_rules_add_no_tint_or_literal_colour() {
    let css = css();
    for selector in [
        "r-empty",
        "r-empty .what",
        "r-empty .next",
        "r-error",
        "r-error .what",
        "r-error .why",
        "r-error .next",
    ] {
        let body = rule_body(&css, selector).unwrap();
        assert!(
            !body.contains("color-mix") && !body.contains('#'),
            "{selector}"
        );
    }
}

#[test]
fn state_lines_stack_as_blocks() {
    let css = css();
    for selector in [
        "r-empty .what",
        "r-empty .next",
        "r-error .what",
        "r-error .why",
        "r-error .next",
    ] {
        assert_declares(&css, selector, "display: block;");
    }
}

fn function_body(js: &str, name: &str) -> String {
    let start = js
        .find(&format!("function {name}("))
        .unwrap_or_else(|| panic!("no function {name}"));
    let rest = &js[start..];
    let end = rest.find("\n}\n").unwrap_or(rest.len());
    rest[..end].to_string()
}

#[test]
fn function_body_reads_one_top_level_function() {
    let js = "function a() {\n  1;\n}\n\nfunction b() {\n  2;\n}\n";
    assert_eq!(function_body(js, "a"), "function a() {\n  1;");
}

#[test]
fn failure_builds_the_what_why_next_markup_with_a_btn_retry() {
    let body = function_body(&script(), "failure");
    assert!(body.contains(r#"class: "btn""#), "{body}");
    assert!(body.contains(r#"class: "what""#), "{body}");
    assert!(body.contains(r#"class: "why""#), "{body}");
    assert!(body.contains(r#"class: "next""#), "{body}");
    assert!(body.contains(r#"role: "alert""#), "{body}");
}

#[test]
fn failure_sets_the_server_message_as_text_only() {
    let js = script();
    let body = function_body(&js, "failure");
    assert!(body.contains("why.textContent ="), "{body}");
    assert!(!body.contains("innerHTML"), "{body}");
    for line in js.lines().filter(|line| line.contains("why")) {
        assert!(!line.contains("innerHTML"), "{line}");
    }
}

#[test]
fn every_failure_call_passes_a_retry() {
    let js = script();
    for name in ["show", "openNode", "fillRendered"] {
        let body = function_body(&js, name);
        assert!(body.contains("failure(error, "), "{name}");
        assert!(!body.contains("failure(error)"), "{name}");
    }
}

#[test]
fn node_and_text_retries_run_the_same_loader_again() {
    let js = script();
    assert!(function_body(&js, "openNode").contains("openNode(item, ctx, true)"));
    assert!(function_body(&js, "fillRendered").contains("fillRendered(row, markdown, true)"));
    assert!(function_body(&js, "show").contains("show(true)"));
}

#[test]
fn a_retry_moves_focus_to_a_tabindex_target_or_the_new_retry_button() {
    let js = script();
    assert!(function_body(&js, "focusTarget").contains(r#"setAttribute("tabindex", "-1")"#));
    assert!(function_body(&js, "focusTarget").contains(".focus()"));
    assert!(function_body(&js, "focusRetry").contains(".focus()"));
    for name in ["show", "openNode", "fillRendered"] {
        let body = function_body(&js, name);
        assert!(body.contains("focusTarget("), "{name}");
        assert!(body.contains("focusRetry("), "{name}");
    }
}

#[test]
fn a_failed_node_open_keeps_the_item_expanded() {
    let body = function_body(&script(), "toggleItem");
    assert!(
        !body.contains("setExpanded(item, false);\n    return;"),
        "{body}"
    );
    assert!(!body.contains("delete item.dataset.loaded"), "{body}");
}

#[test]
fn empty_states_use_one_helper_with_compact_and_full_forms() {
    let js = script();
    assert_eq!(js.matches(r#"el("r-empty""#).count(), 1);
    assert!(js.contains(r#"emptyState("summary pending", true)"#));
    assert!(js.contains(r#"emptyState("no entries in this page", true)"#));
    assert_eq!(js.matches(r#"emptyState("no entries yet")"#).count(), 2);
    assert!(function_body(&js, "emptyState").contains(r#"class: "what""#));
}

#[test]
fn a_script_built_empty_state_is_a_status_region() {
    let body = function_body(&script(), "emptyState");
    assert!(
        body.contains(r#"el("r-empty", null, { role: "status" })"#),
        "{body}"
    );
}
