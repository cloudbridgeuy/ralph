//! TodoWrite tool result formatting (verbose mode).
//!
//! Formats TodoWrite tool results with confirmation messages.

use ralph_core::stream::ToolResult;

use super::super::processor::StreamProcessor;
use super::super::utils::truncate_string;

/// Format a TodoWrite tool result with verbose output.
///
/// In verbose mode, displays confirmation of the todo update. Since
/// TodoWrite typically doesn't have meaningful result content (just
/// success/failure), we show a summary message.
pub fn format_todowrite_tool_result_verbose(
    processor: &StreamProcessor,
    result: &ToolResult,
) -> String {
    if result.is_error {
        let error_content = result
            .content
            .as_ref()
            .map(|c| truncate_string(c, 200))
            .unwrap_or_else(|| "(todo update failed)".to_string());

        return if processor.highlighting_enabled {
            format!("\x1b[31m✗ TodoWrite error:\x1b[0m {}\n", error_content)
        } else {
            format!("! TodoWrite error: {}\n", error_content)
        };
    }

    // Success case - show confirmation message
    if processor.highlighting_enabled {
        "\x1b[32m✓\x1b[0m \x1b[90mtodos updated\x1b[0m\n".to_string()
    } else {
        "(todos updated)\n".to_string()
    }
}
