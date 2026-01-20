//! Grep tool result formatting (verbose mode).
//!
//! Formats Grep tool results with match highlighting.

use ralph_core::stream::{ToolInvocation, ToolResult};

use super::super::processor::StreamProcessor;
use super::super::utils::truncate_string;
use crate::render::highlight_grep_match;

/// Format a Grep tool result with verbose output.
///
/// In verbose mode, the matched files/content are displayed without truncation
/// and with appropriate coloring for matches.
pub fn format_grep_tool_result_verbose(
    processor: &StreamProcessor,
    invocation: ToolInvocation,
    result: &ToolResult,
) -> String {
    const MAX_RESULT_LINES: usize = 100;

    if result.is_error {
        // Error case - show error message
        let error_content = result
            .content
            .as_ref()
            .map(|c| truncate_string(c, 200))
            .unwrap_or_else(|| "(grep failed)".to_string());

        return if processor.highlighting_enabled {
            format!("\x1b[31m✗ Grep error:\x1b[0m {}\n", error_content)
        } else {
            format!("! Grep error: {}\n", error_content)
        };
    }

    let content = result.content.as_deref().unwrap_or("");

    // Empty result
    if content.is_empty() {
        return if processor.highlighting_enabled {
            "\x1b[90m(no matches)\x1b[0m\n".to_string()
        } else {
            "(no matches)\n".to_string()
        };
    }

    // Get the output mode to determine formatting
    let output_mode = invocation
        .input
        .get("output_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("files_with_matches");

    // Count lines for potential truncation
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let (display_lines, truncated) = if line_count > MAX_RESULT_LINES {
        (&lines[..MAX_RESULT_LINES], true)
    } else {
        (&lines[..], false)
    };

    if processor.highlighting_enabled {
        let mut output = String::new();

        // Results header showing match count
        let match_word = if line_count == 1 { "match" } else { "matches" };
        output.push_str(&format!(
            "\x1b[32m✓\x1b[0m \x1b[90m{} {}\x1b[0m\n",
            line_count, match_word
        ));

        // Format based on output mode
        match output_mode {
            "files_with_matches" => {
                // Just file paths - show them in dim color
                for line in display_lines {
                    output.push_str(&format!("  \x1b[90m{}\x1b[0m\n", line));
                }
            }
            "content" => {
                // Content with line numbers - highlight the pattern
                for line in display_lines {
                    // Format: filename:line_number:content
                    // Try to highlight the matched pattern in the line
                    let highlighted_line = highlight_grep_match(line);
                    output.push_str(&format!("  {}\n", highlighted_line));
                }
            }
            "count" => {
                // Just counts - show path:count pairs
                for line in display_lines {
                    output.push_str(&format!("  \x1b[90m{}\x1b[0m\n", line));
                }
            }
            _ => {
                // Unknown mode - show raw
                for line in display_lines {
                    output.push_str(&format!("  \x1b[90m{}\x1b[0m\n", line));
                }
            }
        }

        if truncated {
            output.push_str(&format!(
                "\x1b[90m... {} more lines\x1b[0m\n",
                line_count - MAX_RESULT_LINES
            ));
        }

        output
    } else {
        // Plain text format
        let mut output = String::new();

        let match_word = if line_count == 1 { "match" } else { "matches" };
        output.push_str(&format!("{} {}\n", line_count, match_word));

        for line in display_lines {
            output.push_str(&format!("  {}\n", line));
        }

        if truncated {
            output.push_str(&format!(
                "... {} more lines\n",
                line_count - MAX_RESULT_LINES
            ));
        }

        output
    }
}

// Tests for highlight_grep_match are in crate::render::utils
