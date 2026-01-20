//! TodoWrite tool result formatting (verbose mode).
//!
//! Formats TodoWrite tool results with confirmation messages.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::ToolResult;

use crate::render::{render_todowrite_result, RenderContext};

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
    // For errors, use truncated content as message
    let message = if result.is_error {
        result.content.as_ref().map(|c| truncate_string(c, 200))
    } else {
        Some("todos updated".to_string())
    };

    // Use shared renderer with processor's highlighter
    let ctx = if processor.highlighting_enabled {
        RenderContext::terminal(&processor.code_highlighter)
    } else {
        RenderContext::plain(&processor.code_highlighter)
    };

    render_todowrite_result(&ctx, result.is_error, message.as_deref())
}
