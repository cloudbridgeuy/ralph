//! Grep tool invocation formatting (verbose mode).
//!
//! Formats Grep tool invocations with detailed parameter display.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::ToolInvocation;

use crate::render::{render_grep_invocation, GrepInvocationParams};

use super::super::processor::StreamProcessor;
use super::super::types::GrepParams;

/// Format a Grep tool invocation with verbose output.
///
/// In verbose mode, the pattern is shown with regex syntax highlighting
/// and additional search parameters are displayed.
pub fn format_grep_tool_invocation_verbose(
    processor: &StreamProcessor,
    invocation: &ToolInvocation,
) -> String {
    let params = GrepParams::from_invocation_input(&invocation.input);

    // Build params struct for shared renderer
    let render_params = GrepInvocationParams {
        pattern: &params.pattern,
        path: params.path.as_deref(),
        output_mode: params.output_mode.as_deref(),
        glob: params.glob.as_deref(),
        file_type: params.file_type.as_deref(),
        case_insensitive: params.case_insensitive,
    };

    render_grep_invocation(&processor.render_context(), &render_params)
}
