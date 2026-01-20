//! Glob tool result formatting (verbose mode).
//!
//! Formats Glob tool results with all matched files displayed.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::ToolResult;

use crate::render::{render_glob_result, RenderContext};

use super::super::processor::StreamProcessor;
use super::super::utils::truncate_string;

/// Format a Glob tool result with verbose output.
///
/// In verbose mode, all matched files are displayed without truncation,
/// grouped by directory for readability, with a total match count.
pub fn format_glob_tool_result_verbose(processor: &StreamProcessor, result: &ToolResult) -> String {
    if result.is_error {
        // Error case - show error message
        let error_content = result
            .content
            .as_ref()
            .map(|c| truncate_string(c, 200))
            .unwrap_or_else(|| "(glob failed)".to_string());

        return if processor.highlighting_enabled {
            format!("\x1b[31m✗ Glob error:\x1b[0m {}\n", error_content)
        } else {
            format!("! Glob error: {}\n", error_content)
        };
    }

    let content = result.content.as_deref().unwrap_or("");

    // Count files
    let file_count = content.lines().filter(|l| !l.is_empty()).count();

    // Check for truncation indicators
    let truncated = content.contains("... (truncated)") || content.contains("Output truncated");

    // Use shared renderer with processor's highlighter
    let ctx = if processor.highlighting_enabled {
        RenderContext::terminal(&processor.code_highlighter)
    } else {
        RenderContext::plain(&processor.code_highlighter)
    };

    render_glob_result(&ctx, file_count, content, truncated)
}

// Tests for group_files_by_directory are in crate::render::utils
