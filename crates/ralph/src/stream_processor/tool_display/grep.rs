//! Grep tool invocation formatting (verbose mode).
//!
//! Formats Grep tool invocations with detailed parameter display.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::ToolInvocation;

use crate::render::{render_grep_invocation, GrepInvocationParams, RenderContext};

use super::super::processor::StreamProcessor;

/// Format a Grep tool invocation with verbose output.
///
/// In verbose mode, the pattern is shown with regex syntax highlighting
/// and additional search parameters are displayed.
pub fn format_grep_tool_invocation_verbose(
    processor: &StreamProcessor,
    invocation: &ToolInvocation,
) -> String {
    // Extract parameters from the invocation
    let pattern = invocation
        .input
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let path = invocation.input.get("path").and_then(|v| v.as_str());

    let output_mode = invocation.input.get("output_mode").and_then(|v| v.as_str());

    let glob = invocation.input.get("glob").and_then(|v| v.as_str());

    let file_type = invocation.input.get("type").and_then(|v| v.as_str());

    let case_insensitive = invocation
        .input
        .get("-i")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Build params struct for shared renderer
    let params = GrepInvocationParams {
        pattern,
        path,
        output_mode,
        glob,
        file_type,
        case_insensitive,
    };

    // Use shared renderer with processor's highlighter
    let ctx = if processor.highlighting_enabled {
        RenderContext::terminal(&processor.code_highlighter)
    } else {
        RenderContext::plain(&processor.code_highlighter)
    };

    render_grep_invocation(&ctx, &params)
}
