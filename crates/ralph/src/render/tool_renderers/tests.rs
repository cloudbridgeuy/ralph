//! Unit tests for tool rendering functions.

use super::*;
use crate::highlight::{Highlighter, ThemeConfig};

fn test_highlighter() -> Highlighter {
    Highlighter::with_config(ThemeConfig::default()).unwrap()
}

// =========================================================================
// Bash Invocation Tests
// =========================================================================

#[test]
fn test_render_bash_invocation_single_line_terminal() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::terminal(&highlighter);
    let output = render_bash_invocation(&ctx, "ls -la");

    assert!(
        output.contains("▶ Bash"),
        "Missing header. Output: {:?}",
        output
    );
    // Command parts may be split by ANSI codes from syntax highlighting
    assert!(output.contains("ls"), "Missing 'ls'. Output: {:?}", output);
    assert!(output.contains("la"), "Missing 'la'. Output: {:?}", output);
    assert!(output.contains("\x1b[")); // ANSI codes
}

#[test]
fn test_render_bash_invocation_single_line_plain() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let output = render_bash_invocation(&ctx, "ls -la");

    assert!(output.contains("> Bash"));
    assert!(output.contains("ls -la"));
    assert!(!output.contains("\x1b[")); // No ANSI codes
}

#[test]
fn test_render_bash_invocation_multiline() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let output = render_bash_invocation(&ctx, "echo 'line1'\necho 'line2'");

    assert!(output.contains("> Bash"));
    assert!(output.contains("```sh"));
    assert!(output.contains("echo 'line1'"));
}

// =========================================================================
// Grep Invocation Tests
// =========================================================================

#[test]
fn test_render_grep_invocation_minimal() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let params = GrepInvocationParams {
        pattern: "fn main",
        path: None,
        output_mode: None,
        glob: None,
        file_type: None,
        case_insensitive: false,
    };
    let output = render_grep_invocation(&ctx, &params);

    assert!(output.contains("> Grep"));
    assert!(output.contains("Pattern: fn main"));
}

#[test]
fn test_render_grep_invocation_full() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let params = GrepInvocationParams {
        pattern: "TODO",
        path: Some("src/"),
        output_mode: Some("content"),
        glob: Some("*.rs"),
        file_type: Some("rust"),
        case_insensitive: true,
    };
    let output = render_grep_invocation(&ctx, &params);

    assert!(output.contains("Pattern: TODO"));
    assert!(output.contains("Path: src/"));
    assert!(output.contains("Mode: content"));
    assert!(output.contains("glob: *.rs"));
    assert!(output.contains("type: rust"));
    assert!(output.contains("case-insensitive: true"));
}

// =========================================================================
// Read Result Tests
// =========================================================================

#[test]
fn test_render_read_result_empty() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let output = render_read_result(&ctx, "test.rs", "", 0, false);

    assert!(output.contains("(empty file)"));
}

#[test]
fn test_render_read_result_normalizes_cat_n() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let output = render_read_result(&ctx, "test.rs", "     1\tfn main() {}", 1, false);

    assert!(output.contains("1 │ fn main() {}"));
    assert!(output.contains("1 line"));
}

// =========================================================================
// Glob Result Tests
// =========================================================================

#[test]
fn test_render_glob_result_empty() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let output = render_glob_result(&ctx, 0, "", false);

    assert!(output.contains("(no matches)"));
}

#[test]
fn test_render_glob_result_grouped() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let output = render_glob_result(&ctx, 3, "src/main.rs\nsrc/lib.rs\ntests/test.rs", false);

    assert!(output.contains("3 files matched"));
    assert!(output.contains("src/"));
    assert!(output.contains("tests/"));
}

// =========================================================================
// Edit Before/After Tests
// =========================================================================

#[test]
fn test_render_edit_before_after_plain() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let output = render_edit_before_after(&ctx, "test.rs", "let x = 1;", "let x = 2;");

    assert!(output.contains("test.rs"));
    assert!(output.contains("let x = 1;"));
    assert!(output.contains("────────────────────"));
    assert!(output.contains("let x = 2;"));
}

// =========================================================================
// TodoWrite Tests
// =========================================================================

#[test]
fn test_render_todowrite_invocation() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let todos = vec![
        TodoDisplayItem {
            content: "Task 1",
            status: "completed",
            active_form: None,
        },
        TodoDisplayItem {
            content: "Task 2",
            status: "in_progress",
            active_form: Some("Working on Task 2"),
        },
        TodoDisplayItem {
            content: "Task 3",
            status: "pending",
            active_form: None,
        },
    ];
    let output = render_todowrite_invocation(&ctx, &todos);

    assert!(output.contains("> TodoWrite"));
    assert!(output.contains("[x] Task 1"));
    assert!(output.contains("[~] Task 2 (Working on Task 2)"));
    assert!(output.contains("[ ] Task 3"));
}

#[test]
fn test_render_todowrite_result_success() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let output = render_todowrite_result(&ctx, false, Some("Todos updated"));

    assert!(output.contains("Todos updated"));
    assert!(!output.contains("!"));
}

#[test]
fn test_render_todowrite_result_error() {
    let highlighter = test_highlighter();
    let ctx = RenderContext::plain(&highlighter);
    let output = render_todowrite_result(&ctx, true, Some("Failed"));

    assert!(output.contains("! Failed"));
}
