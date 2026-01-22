//! Glob tool invocation formatting (verbose mode).
//!
//! Formats Glob tool invocations with detailed parameter display.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::ToolInvocation;

use crate::render::render_glob_invocation;

use super::super::processor::StreamProcessor;

/// Format a Glob tool invocation with verbose output.
///
/// In verbose mode, the full glob pattern is shown without truncation
/// and additional search parameters are displayed.
pub fn format_glob_tool_invocation_verbose(
    processor: &StreamProcessor,
    invocation: &ToolInvocation,
) -> String {
    // Extract the pattern from the input
    let pattern = invocation
        .input
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("*");

    // Extract optional search path
    let path = invocation.input.get("path").and_then(|v| v.as_str());

    render_glob_invocation(&processor.render_context(), pattern, path)
}
