//! Edit tool result formatting.
//!
//! Formats Edit tool results containing diffs with syntax highlighting.

use crate::diff_highlight::highlight_with_basic_colors;
use ralph_core::stream::ToolInvocation;

use super::super::processor::StreamProcessor;

/// Format an Edit tool result that contains a diff with syntax highlighting.
///
/// This displays:
/// 1. A file path header showing which file was edited
/// 2. The diff content with syntax highlighting (green for additions, red for deletions)
/// 3. Truncation indicator if the diff is very long
pub fn format_edit_diff_result(
    processor: &StreamProcessor,
    invocation: ToolInvocation,
    diff_content: &str,
) -> String {
    // Extract file path from the invocation input
    let file_path = invocation
        .input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown file");

    // Count lines for potential truncation
    let lines: Vec<&str> = diff_content.lines().collect();
    let line_count = lines.len();
    const MAX_DIFF_LINES: usize = 50;

    // Truncate if too long
    let (display_content, truncated) = if line_count > MAX_DIFF_LINES {
        let truncated_lines: String = lines[..MAX_DIFF_LINES].join("\n");
        (truncated_lines, true)
    } else {
        (diff_content.to_string(), false)
    };

    if processor.highlighting_enabled {
        // Highlight the diff
        let highlighted_diff = highlight_with_basic_colors(&display_content);

        // Build output with header
        let mut output = String::new();

        // File path header with box drawing
        output.push_str(&format!("\x1b[36m── {} ──\x1b[0m\n", file_path));

        // The highlighted diff content wrapped in diff fences
        output.push_str("```diff\n");
        output.push_str(&highlighted_diff);
        if !highlighted_diff.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("```\n");

        // Truncation indicator
        if truncated {
            output.push_str(&format!(
                "\x1b[90m... {} more lines\x1b[0m\n",
                line_count - MAX_DIFF_LINES
            ));
        }

        output
    } else {
        // Plain text format
        let mut output = String::new();

        // Simple header
        output.push_str(&format!("-- {} --\n", file_path));

        // Plain diff content
        output.push_str("```diff\n");
        output.push_str(&display_content);
        if !display_content.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("```\n");

        // Truncation indicator
        if truncated {
            output.push_str(&format!("... {} more lines\n", line_count - MAX_DIFF_LINES));
        }

        output
    }
}
