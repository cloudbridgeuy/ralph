//! Bash tool result formatting.
//!
//! Formats Bash tool execution results with output styling.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::ToolResult;

use crate::render::{render_bash_result, RenderContext};

use super::super::processor::StreamProcessor;
use super::super::utils::truncate_multiline;

/// Format a Bash tool result with distinct output styling.
///
/// The output is shown in a dimmed/muted color to distinguish it from the command.
/// Exit code is shown if non-zero (error indicator).
/// Very long outputs are truncated with a '... N more lines' indicator.
pub fn format_bash_tool_result(processor: &StreamProcessor, result: &ToolResult) -> String {
    const MAX_OUTPUT_LINES: usize = 30;

    // Truncate content if needed
    let (content, truncated) = if let Some(ref c) = result.content {
        let (truncated_content, was_truncated) = truncate_multiline(c, MAX_OUTPUT_LINES);
        (Some(truncated_content), was_truncated)
    } else {
        (None, false)
    };

    // Use shared renderer with processor's highlighter
    let ctx = if processor.highlighting_enabled {
        RenderContext::terminal(&processor.code_highlighter)
    } else {
        RenderContext::plain(&processor.code_highlighter)
    };

    render_bash_result(&ctx, result.is_error, content.as_deref(), truncated)
}
