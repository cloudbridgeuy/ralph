//! Read tool invocation formatting (verbose mode).
//!
//! Formats Read tool invocations with file path and range information.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::ToolInvocation;

use crate::render::render_read_invocation;

use super::super::processor::StreamProcessor;

/// Format a Read tool invocation with verbose output.
///
/// In verbose mode, the file path is shown clearly with additional context
/// about line offset and limit if provided.
pub fn format_read_tool_invocation_verbose(
    processor: &StreamProcessor,
    invocation: &ToolInvocation,
) -> String {
    // Extract the file path from the input
    let file_path = invocation
        .input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown file)");

    // Extract optional offset and limit
    let offset = invocation.input.get("offset").and_then(|v| v.as_u64());
    let limit = invocation.input.get("limit").and_then(|v| v.as_u64());

    render_read_invocation(&processor.render_context(), file_path, offset, limit)
}
