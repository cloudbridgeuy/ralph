//! Grep tool result formatting (verbose mode).
//!
//! Formats Grep tool results with match highlighting.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::{ToolInvocation, ToolResult};

use crate::render::render_grep_result;

use super::super::processor::StreamProcessor;
use super::super::utils::truncate_string;

/// Format a Grep tool result with verbose output.
///
/// In verbose mode, the matched files/content are displayed without truncation
/// and with appropriate coloring for matches.
pub fn format_grep_tool_result_verbose(
    processor: &StreamProcessor,
    invocation: ToolInvocation,
    result: &ToolResult,
) -> String {
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

    // Get the output mode to determine formatting
    let output_mode = invocation
        .input
        .get("output_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("files_with_matches");

    // Count matches (non-empty lines)
    let match_count = content.lines().filter(|l| !l.is_empty()).count();

    render_grep_result(
        &processor.render_context(),
        match_count,
        output_mode,
        content,
    )
}

// Tests for highlight_grep_match are in crate::render::utils
