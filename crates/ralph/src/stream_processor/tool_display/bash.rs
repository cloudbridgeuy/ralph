//! Bash tool invocation formatting.
//!
//! Formats Bash tool invocations with syntax highlighting.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::ToolInvocation;

use crate::render::{render_bash_invocation, RenderContext};

use super::super::processor::StreamProcessor;

/// Format a Bash tool invocation with syntax highlighting.
///
/// The command is shown in full (not truncated) with shell syntax highlighting
/// applied. Multi-line commands are displayed with proper formatting.
pub fn format_bash_tool_invocation(
    processor: &StreamProcessor,
    invocation: &ToolInvocation,
) -> String {
    // Extract the command from the input
    let command = invocation
        .input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Use shared renderer with processor's highlighter
    let ctx = if processor.highlighting_enabled {
        RenderContext::terminal(&processor.code_highlighter)
    } else {
        RenderContext::plain(&processor.code_highlighter)
    };

    render_bash_invocation(&ctx, command)
}
