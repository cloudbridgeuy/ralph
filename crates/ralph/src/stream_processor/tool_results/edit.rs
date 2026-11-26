//! Edit tool result formatting.
//!
//! Formats Edit tool results with syntax-highlighted before/after display.
//!
//! Claude CLI's Edit tool returns success messages like "File updated", not actual
//! diff content. To show meaningful changes, we:
//! 1. Capture old_string and new_string from Edit tool input (via EditSnapshot)
//! 2. Display before block (old_string) with red background + syntax highlighting
//! 3. Display after block (new_string) with green background + syntax highlighting
//! 4. Fall back to unified diff if old_string/new_string unavailable

use crate::render::ansi;
use crate::render::code_frame::framed_diff;
use ralph_core::code_frame::Label;

use crate::render::extract_language_from_path;
use crate::render::RenderContext;

/// Maximum lines to show in a before/after block before truncating.
const MAX_BLOCK_LINES: usize = 30;

/// Maximum lines to show in a unified diff before truncating.
const MAX_DIFF_LINES: usize = 50;

pub fn format_before_after(
    ctx: &RenderContext,
    file_path: &str,
    old_str: &str,
    new_str: &str,
) -> String {
    let language = extract_language_from_path(file_path);
    if ctx.terminal {
        format_before_after_highlighted(ctx, file_path, old_str, new_str, language)
    } else {
        format_before_after_plain(file_path, old_str, new_str)
    }
}

/// Format before/after blocks with syntax highlighting and colored backgrounds.
fn format_before_after_highlighted(
    ctx: &RenderContext,
    file_path: &str,
    old_str: &str,
    new_str: &str,
    language: Option<&'static str>,
) -> String {
    let mut output = String::new();

    // File path header with box drawing
    output.push_str(&format!(
        "{}── {} ──{}\n",
        ctx.tools.name_fg(),
        file_path,
        ansi::RESET
    ));

    // Before block
    output.push_str(&format_highlighted_block(
        ctx,
        "Before:",
        old_str,
        language,
        &ansi::bg(Some(ctx.diff.del_bg)),
    ));

    // Horizontal separator
    output.push_str(&format!(
        "{}─────────────────────────{}\n",
        ctx.tools.label_fg(),
        ansi::RESET
    ));

    // After block
    output.push_str(&format_highlighted_block(
        ctx,
        "After:",
        new_str,
        language,
        &ansi::bg(Some(ctx.diff.add_bg)),
    ));

    output
}

/// Format a single highlighted block (before or after) with background color.
fn format_highlighted_block(
    ctx: &RenderContext,
    label: &str,
    content: &str,
    language: Option<&'static str>,
    bg_color: &str,
) -> String {
    let mut output = String::new();

    // Label
    output.push_str(&format!(
        "{}{}{}\n",
        ctx.tools.label_fg(),
        label,
        ansi::RESET
    ));

    // Handle empty content
    if content.is_empty() {
        output.push_str(&format!("{}(empty){}\n", ctx.tools.label_fg(), ansi::RESET));
        return output;
    }

    // Count and potentially truncate lines
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let (display_lines, truncated) = if line_count > MAX_BLOCK_LINES {
        (&lines[..MAX_BLOCK_LINES], true)
    } else {
        (&lines[..], false)
    };

    let display_content = display_lines.join("\n");

    // Apply syntax highlighting first (if language is detected)
    let highlighted = if let Some(lang) = language {
        ctx.highlighter.highlight(&display_content, Some(lang))
    } else {
        display_content.clone()
    };

    // Apply background color to line numbers only, syntax highlighting to content
    let line_count_width = display_lines.len().to_string().len().max(2);
    for (i, line) in highlighted.lines().enumerate() {
        let line_num = i + 1;
        // Format: line number with background │ content with syntax highlighting only
        output.push_str(&format!(
            "{}{:>width$} {}│{} {}\n",
            bg_color,
            line_num,
            ansi::RESET,
            ansi::RESET,
            line,
            width = line_count_width
        ));
    }

    // Truncation indicator
    if truncated {
        output.push_str(&format!(
            "{}... {} more lines{}\n",
            ctx.tools.label_fg(),
            line_count - MAX_BLOCK_LINES,
            ansi::RESET
        ));
    }

    output
}

/// Format before/after blocks in plain text (no ANSI codes).
fn format_before_after_plain(file_path: &str, old_str: &str, new_str: &str) -> String {
    let mut output = String::new();

    // File path header
    output.push_str(&format!("-- {} --\n", file_path));

    // Before block
    output.push_str(&format_plain_block("Before:", old_str));

    // Horizontal separator
    output.push_str("-------------------------\n");

    // After block
    output.push_str(&format_plain_block("After:", new_str));

    output
}

/// Format a single plain text block (before or after).
fn format_plain_block(label: &str, content: &str) -> String {
    let mut output = String::new();

    output.push_str(&format!("{}\n", label));

    if content.is_empty() {
        output.push_str("(empty)\n");
        return output;
    }

    // Count and potentially truncate lines
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let (display_lines, truncated) = if line_count > MAX_BLOCK_LINES {
        (&lines[..MAX_BLOCK_LINES], true)
    } else {
        (&lines[..], false)
    };

    let line_count_width = display_lines.len().to_string().len().max(2);
    for (i, line) in display_lines.iter().enumerate() {
        let line_num = i + 1;
        output.push_str(&format!(
            "{:>width$} │ {}\n",
            line_num,
            line,
            width = line_count_width
        ));
    }

    if truncated {
        output.push_str(&format!(
            "... {} more lines\n",
            line_count - MAX_BLOCK_LINES
        ));
    }

    output
}

/// Format a simple "no changes" message.
pub fn format_no_changes_message(ctx: &RenderContext, file_path: &str) -> String {
    if ctx.terminal {
        format!(
            "{}✓{} {}Edit: {} (no changes){}\n",
            ctx.tools.ok_fg(),
            ansi::RESET,
            ctx.tools.label_fg(),
            file_path,
            ansi::RESET
        )
    } else {
        format!("  Edit: {} (no changes)\n", file_path)
    }
}

pub fn format_diff_output(ctx: &RenderContext, file_path: &str, diff_content: &str) -> String {
    let lines: Vec<&str> = diff_content.lines().collect();
    let line_count = lines.len();

    let (display_content, truncated) = if line_count > MAX_DIFF_LINES {
        let truncated_lines: String = lines[..MAX_DIFF_LINES].join("\n");
        (truncated_lines, true)
    } else {
        (diff_content.to_string(), false)
    };

    if ctx.terminal {
        let ink = ctx.tools.name_fg();
        let head = Label::File {
            path: file_path,
            kind: "diff",
            ink: &ink,
        };
        let mut output = framed_diff(ctx, &head, &display_content);

        if truncated {
            output.push_str(&format!(
                "{}... {} more lines{}\n",
                ctx.tools.label_fg(),
                line_count - MAX_DIFF_LINES,
                ansi::RESET
            ));
        }

        output
    } else {
        let mut output = String::new();

        output.push_str(&format!("-- {} --\n", file_path));

        output.push_str("```diff\n");
        output.push_str(&display_content);
        if !display_content.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("```\n");

        if truncated {
            output.push_str(&format!("... {} more lines\n", line_count - MAX_DIFF_LINES));
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_processor::StreamProcessor;

    #[test]
    fn test_format_diff_output_truncation() {
        let processor = StreamProcessor::with_highlighting(false);
        let long_diff: String = (0..100).map(|i| format!("+line{}\n", i)).collect();

        let output = format_diff_output(&processor.render_context(), "test.txt", &long_diff);

        assert!(output.contains("... 50 more lines"));
    }

    #[test]
    fn test_format_diff_output_no_truncation() {
        let processor = StreamProcessor::with_highlighting(false);
        let short_diff = "+line1\n+line2\n";

        let output = format_diff_output(&processor.render_context(), "test.txt", short_diff);

        assert!(!output.contains("more lines"));
    }

    #[test]
    fn test_format_before_after_plain() {
        let output = format_before_after_plain("test.rs", "old code", "new code");

        assert!(output.contains("-- test.rs --"));
        assert!(output.contains("Before:"));
        assert!(output.contains("old code"));
        assert!(output.contains("After:"));
        assert!(output.contains("new code"));
    }

    #[test]
    fn test_format_before_after_plain_empty_old() {
        let output = format_before_after_plain("test.rs", "", "new code");

        assert!(output.contains("(empty)"));
        assert!(output.contains("new code"));
    }

    #[test]
    fn test_format_before_after_plain_empty_new() {
        let output = format_before_after_plain("test.rs", "old code", "");

        assert!(output.contains("old code"));
        assert!(output.contains("(empty)"));
    }

    #[test]
    fn test_format_plain_block_truncation() {
        let long_content: String = (0..50)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");

        let output = format_plain_block("Test:", &long_content);

        assert!(output.contains("... 20 more lines"));
    }

    #[test]
    fn test_format_plain_block_no_truncation() {
        let short_content = "line1\nline2\nline3";

        let output = format_plain_block("Test:", short_content);

        assert!(!output.contains("more lines"));
        assert!(output.contains("line1"));
        assert!(output.contains("line3"));
    }

    #[test]
    fn test_format_before_after_highlighted_has_ansi_codes() {
        let processor = StreamProcessor::with_highlighting(true);

        let output = format_before_after_highlighted(
            &processor.render_context(),
            "test.rs",
            "let x = 1;",
            "let x = 2;",
            Some("rust"),
        );

        // Should contain ANSI escape codes
        assert!(output.contains("\x1b["));
        // Should contain the file path
        assert!(output.contains("test.rs"));
        // Should contain "Before:" and "After:" labels
        assert!(output.contains("Before:"));
        assert!(output.contains("After:"));
        // Should contain the background color codes
        assert!(output.contains(ansi::RED_BG) || output.contains(ansi::GREEN_BG));
    }

    #[test]
    fn test_format_no_changes_message_highlighted() {
        let processor = StreamProcessor::with_highlighting(true);

        let output = format_no_changes_message(&processor.render_context(), "test.rs");

        assert!(output.contains("✓"));
        assert!(output.contains("test.rs"));
        assert!(output.contains("no changes"));
    }

    #[test]
    fn test_format_no_changes_message_plain() {
        let processor = StreamProcessor::with_highlighting(false);

        let output = format_no_changes_message(&processor.render_context(), "test.rs");

        assert!(output.contains("test.rs"));
        assert!(output.contains("no changes"));
        // Should not contain ANSI codes
        assert!(!output.contains("\x1b["));
    }

    fn colored_processor() -> StreamProcessor {
        let mut processor = StreamProcessor::with_highlighting(true);
        processor.renderer.diff_colors = crate::diff_theme::test_colors();
        processor
    }

    #[test]
    fn before_after_uses_the_configured_backgrounds() {
        let out = format_before_after_highlighted(
            &colored_processor().render_context(),
            "a.rs",
            "x",
            "y",
            None,
        );
        assert!(out.contains("\x1b[48;2;4;5;6m"));
        assert!(out.contains("\x1b[48;2;1;2;3m"));
        assert!(!out.contains(ansi::RED_BG));
        assert!(!out.contains(ansi::GREEN_BG));
    }

    #[test]
    fn unified_diff_uses_the_configured_foregrounds() {
        let out = format_diff_output(&colored_processor().render_context(), "a.rs", "+x\n-y");
        assert!(out.contains("\x1b[94m+x"));
        assert!(out.contains("\x1b[95m-y"));
    }

    #[test]
    fn plain_processor_has_no_escape_byte() {
        let processor = StreamProcessor::with_highlighting(false);
        assert!(
            !format_diff_output(&processor.render_context(), "a.rs", "+x\n-y\n@@ -1 @@")
                .contains('\x1b')
        );
        assert!(
            !format_before_after(&processor.render_context(), "a.rs", "x", "y").contains('\x1b')
        );
    }

    #[test]
    fn edit_result_rule_carries_path() {
        let processor = colored_processor();
        let out = format_diff_output(&processor.render_context(), "src/a.rs", "+x\n");
        let plain = crate::frame::strip_ansi(&out);
        assert!(plain.starts_with("── src/a.rs · diff ─"), "{plain:?}");
        assert!(!plain.contains("```"), "{plain:?}");
        assert_eq!(plain.lines().filter(|row| row.contains('─')).count(), 1);
        assert!(plain.ends_with("  \n  +x\n  \n"), "{plain:?}");
    }

    #[test]
    fn truncated_edit_result_counts_lines_after_the_block() {
        let processor = colored_processor();
        let diff = "+x\n".repeat(MAX_DIFF_LINES + 3);
        let out = format_diff_output(&processor.render_context(), "a.rs", &diff);
        let plain = crate::frame::strip_ansi(&out);
        assert!(plain.ends_with("  +x\n  \n... 3 more lines\n"), "{plain:?}");
    }

    #[test]
    fn headers_and_marks_use_the_configured_tool_colors() {
        let mut processor = colored_processor();
        processor.renderer.tool_colors = crate::tool_theme::test_colors();
        let out =
            format_before_after_highlighted(&processor.render_context(), "a.rs", "x", "y", None);
        assert!(out.starts_with("\x1b[94m── a.rs"));
        assert!(out.contains("\x1b[96mBefore:"));
        assert!(
            format_no_changes_message(&processor.render_context(), "a.rs").starts_with("\x1b[97m✓")
        );
    }
}
