//! Read tool result rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::utils::{extract_language_from_path, normalize_cat_n_format};

/// Render a Read tool result (verbose mode).
pub fn render_read_result(
    ctx: &RenderContext,
    file_path: &str,
    content: &str,
    line_count: usize,
    truncated: bool,
) -> String {
    const MAX_CONTENT_LINES: usize = 100;

    // Empty result
    if content.is_empty() {
        return if ctx.terminal {
            format!("{}(empty file){}\n", ansi::DIM, ansi::RESET)
        } else {
            "(empty file)\n".to_string()
        };
    }

    // Check for binary file indicator
    if content.contains("(binary file)") || content.starts_with('\u{0}') {
        return if ctx.terminal {
            format!("{}(binary file){}\n", ansi::DIM, ansi::RESET)
        } else {
            "(binary file)\n".to_string()
        };
    }

    // Normalize cat-n format before processing
    let normalized_content = normalize_cat_n_format(content);

    // Count lines for potential truncation
    let lines: Vec<&str> = normalized_content.lines().collect();
    let actual_line_count = lines.len();
    let (display_lines, should_truncate) = if actual_line_count > MAX_CONTENT_LINES {
        (&lines[..MAX_CONTENT_LINES], true)
    } else {
        (&lines[..], truncated)
    };

    let language = extract_language_from_path(file_path);
    let mut output = String::new();

    if ctx.terminal {
        // Results header showing line count
        let line_word = if line_count == 1 { "line" } else { "lines" };
        output.push_str(&format!(
            "{}✓{} {}{} {}{}\n",
            ansi::GREEN,
            ansi::RESET,
            ansi::DIM,
            line_count,
            line_word,
            ansi::RESET
        ));

        // Apply syntax highlighting to the content
        let content_to_highlight = display_lines.join("\n");
        let highlighted = if language.is_some() {
            ctx.highlighter.highlight(&content_to_highlight, language)
        } else {
            content_to_highlight.clone()
        };

        // Display highlighted content with indentation
        for line in highlighted.lines() {
            output.push_str(&format!("  {}\n", line));
        }

        if should_truncate {
            output.push_str(&format!(
                "{}... {} more lines{}\n",
                ansi::DIM,
                actual_line_count.saturating_sub(MAX_CONTENT_LINES),
                ansi::RESET
            ));
        }
    } else {
        // Plain text format
        let line_word = if line_count == 1 { "line" } else { "lines" };
        output.push_str(&format!("{} {}\n", line_count, line_word));

        for line in display_lines {
            output.push_str(&format!("  {}\n", line));
        }

        if should_truncate {
            output.push_str(&format!(
                "... {} more lines\n",
                actual_line_count.saturating_sub(MAX_CONTENT_LINES)
            ));
        }
    }

    output
}
