//! Edit tool result rendering.

use crate::diff_highlight::highlight_with_basic_colors;
use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::utils::extract_language_from_path;

/// Render content with colored line numbers (for before/after blocks).
pub fn render_content_block(
    ctx: &RenderContext,
    content: &str,
    language: Option<&str>,
    is_before: bool,
) -> String {
    let mut output = String::new();
    let lines: Vec<&str> = content.lines().collect();
    let max_line_width = lines.len().to_string().len().max(3);

    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        let line_num_str = format!("{:>width$}", line_num, width = max_line_width);

        if ctx.terminal {
            // Line number with background color
            let bg_color = ansi::bg(Some(if is_before {
                ctx.diff.del_bg
            } else {
                ctx.diff.add_bg
            }));
            output.push_str(&format!("{}{} {} ", bg_color, line_num_str, ansi::RESET));

            // Content with syntax highlighting
            let highlighted = if let Some(lang) = language {
                ctx.highlighter.highlight(line, Some(lang))
            } else {
                line.to_string()
            };
            output.push_str(highlighted.trim_end_matches(ansi::RESET));
            output.push_str(&format!("{}\n", ansi::RESET));
        } else {
            output.push_str(&format!("{} │ {}\n", line_num_str, line));
        }
    }

    output
}

/// Render an Edit result with before/after blocks.
pub fn render_edit_before_after(
    ctx: &RenderContext,
    file_path: &str,
    old_content: &str,
    new_content: &str,
) -> String {
    let mut output = String::new();

    // File header
    if ctx.terminal {
        output.push_str(&format!("{}{}{}\n", ansi::GREY, file_path, ansi::RESET));
    } else {
        output.push_str(&format!("{}\n", file_path));
    }

    // Detect language for syntax highlighting
    let language = extract_language_from_path(file_path);

    // Before block
    output.push_str(&render_content_block(ctx, old_content, language, true));

    // Separator
    if ctx.terminal {
        output.push_str(&format!(
            "{}────────────────────{}\n",
            ansi::GREY,
            ansi::RESET
        ));
    } else {
        output.push_str("────────────────────\n");
    }

    // After block
    output.push_str(&render_content_block(ctx, new_content, language, false));

    output
}

/// Render an Edit diff result.
pub fn render_edit_diff(ctx: &RenderContext, file_path: &str, diff_content: &str) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!("{}{}{}\n", ansi::GREY, file_path, ansi::RESET));
        output.push_str(&highlight_with_basic_colors(diff_content, &ctx.diff));
    } else {
        output.push_str(&format!("{}\n", file_path));
        output.push_str(diff_content);
    }
    output.push('\n');

    output
}

/// Render a "no changes" message for Edit/Write tools.
pub fn render_no_changes_message(ctx: &RenderContext, file_path: &str, tool: &str) -> String {
    if ctx.terminal {
        format!(
            "{}{}{}\n{}⚠ No changes ({}){}\n",
            ansi::GREY,
            file_path,
            ansi::RESET,
            ansi::YELLOW,
            tool,
            ansi::RESET
        )
    } else {
        format!("{}\nNo changes ({})\n", file_path, tool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_theme::test_colors;
    use crate::highlight::Highlighter;

    #[test]
    fn diff_result_uses_the_context_diff_colors() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_diff(test_colors());
        assert!(render_edit_diff(&ctx, "a.rs", "+x\n-y").contains("\x1b[94m+x"));
    }

    #[test]
    fn content_block_uses_the_context_background_colors() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_diff(test_colors());
        assert!(render_content_block(&ctx, "x", None, true).contains("\x1b[48;2;4;5;6m"));
        assert!(render_content_block(&ctx, "x", None, false).contains("\x1b[48;2;1;2;3m"));
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        assert!(!render_edit_diff(&ctx, "a.rs", "+x\n-y\n@@ -1 @@").contains('\x1b'));
        assert!(!render_content_block(&ctx, "x", Some("rust"), true).contains('\x1b'));
        assert!(!render_edit_before_after(&ctx, "a.rs", "x", "y").contains('\x1b'));
    }
}
