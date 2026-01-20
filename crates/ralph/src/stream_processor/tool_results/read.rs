//! Read tool result formatting (verbose mode).
//!
//! Formats Read tool results with syntax highlighting.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::{ToolInvocation, ToolResult};

use crate::render::{render_read_result, RenderContext};

use super::super::processor::StreamProcessor;
use super::super::utils::truncate_string;

/// Format a Read tool result with verbose output.
///
/// In verbose mode, the file content is displayed with syntax highlighting
/// based on the file extension. Line numbers from Claude CLI's `cat -n` format
/// are normalized to use pipe separators (e.g., `1 │ content` instead of `     1→content`).
pub fn format_read_tool_result_verbose(
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
            .unwrap_or_else(|| "(read failed)".to_string());

        return if processor.highlighting_enabled {
            format!("\x1b[31m✗ Read error:\x1b[0m {}\n", error_content)
        } else {
            format!("! Read error: {}\n", error_content)
        };
    }

    let content = result.content.as_deref().unwrap_or("");

    // Extract file path for language detection
    let file_path = invocation
        .input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Count lines
    let line_count = content.lines().count();

    // Check for truncation indicators
    let truncated = content.contains("... (truncated)") || content.contains("Output truncated");

    // Use shared renderer with processor's highlighter
    let ctx = if processor.highlighting_enabled {
        RenderContext::terminal(&processor.code_highlighter)
    } else {
        RenderContext::plain(&processor.code_highlighter)
    };

    render_read_result(&ctx, file_path, content, line_count, truncated)
}

// Tests for normalize_cat_n_format and extract_line_number are in crate::render::utils
