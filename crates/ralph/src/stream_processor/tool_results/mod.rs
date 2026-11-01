//! Tool result display formatting.
//!
//! This module handles formatting tool execution results for terminal display.
//! Each tool type can have specialized result formatting, with a default fallback
//! for unknown tools.

mod bash;
mod edit;
mod glob;
mod grep;
mod notebookedit;
mod read;
mod todowrite;
mod write;

use ralph_core::chunk::is_unfenced_diff;
use ralph_core::stream::{ToolInvocation, ToolResult};

use super::processor::StreamProcessor;
use super::utils::truncate_string;
use crate::ansi::RESET;

// Re-export for use in event_handler
pub use edit::format_edit_result_with_snapshot;
pub use notebookedit::format_notebook_result_with_snapshot;
pub use write::format_write_result_with_snapshot;

/// Format a tool result for display with optional context from the original invocation.
///
/// When the original invocation is available and the tool is "Edit", this method
/// will detect if the result contains a diff and apply syntax highlighting.
/// When the tool is "Bash", the output is shown with distinct styling.
/// When the tool is "Grep" and verbose mode is enabled, the results are shown
/// with syntax highlighting.
pub fn format_tool_result_with_context(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: Option<ToolInvocation>,
) -> String {
    // Check for tool-specific formatting
    if let Some(ref inv) = invocation {
        // Edit tool with diff content
        if inv.name == "Edit" && !result.is_error {
            if let Some(ref content) = result.content {
                // Check if content looks like a diff
                if is_unfenced_diff(content) {
                    return edit::format_edit_diff_result(processor, inv.clone(), content);
                }
            }
        }
        // Bash tool with output
        if inv.name == "Bash" {
            return bash::format_bash_tool_result(processor, result);
        }
        // Grep tool with verbose mode
        if inv.name == "Grep" && processor.is_tool_verbose("Grep") {
            return grep::format_grep_tool_result_verbose(processor, inv.clone(), result);
        }
        // Read tool with verbose mode
        if inv.name == "Read" && processor.is_tool_verbose("Read") {
            return read::format_read_tool_result_verbose(processor, inv.clone(), result);
        }
        // TodoWrite tool with verbose mode
        if inv.name == "TodoWrite" && processor.is_tool_verbose("TodoWrite") {
            return todowrite::format_todowrite_tool_result_verbose(processor, result);
        }
        // Glob tool with verbose mode
        if inv.name == "Glob" && processor.is_tool_verbose("Glob") {
            return glob::format_glob_tool_result_verbose(processor, result);
        }
    }

    // Default formatting for other tools
    let truncated_content = result
        .content
        .as_ref()
        .map(|c| truncate_string(c, 200))
        .unwrap_or_else(|| "(no output)".to_string());

    if processor.highlighting_enabled {
        if result.is_error {
            // Red for errors
            format!(
                "{}✗ Error:{RESET} {}\n",
                processor.tool_colors.error_fg(),
                truncated_content
            )
        } else {
            // Green check for success (dim output)
            format!(
                "{}✓{RESET} {}{}{RESET}\n",
                processor.tool_colors.ok_fg(),
                processor.tool_colors.label_fg(),
                truncated_content
            )
        }
    } else {
        // Plain text for non-terminal
        if result.is_error {
            format!("! Error: {}\n", truncated_content)
        } else {
            format!("  {}\n", truncated_content)
        }
    }
}

#[cfg(test)]
mod tool_color_tests {
    use super::*;

    fn processor(color: bool) -> StreamProcessor {
        let mut processor = StreamProcessor::with_highlighting(color);
        processor.tool_colors = crate::tool_theme::test_colors();
        processor
    }

    fn result(is_error: bool) -> ToolResult {
        serde_json::from_value(serde_json::json!({"content": "out", "is_error": is_error})).unwrap()
    }

    #[test]
    fn success_and_error_marks_use_the_configured_colors() {
        let ok = format_tool_result_with_context(&processor(true), &result(false), None);
        assert!(ok.starts_with("\x1b[97m✓"));
        let err = format_tool_result_with_context(&processor(true), &result(true), None);
        assert!(err.starts_with("\x1b[92m✗"));
    }

    #[test]
    fn plain_processor_has_no_escape_byte() {
        for is_error in [false, true] {
            let out = format_tool_result_with_context(&processor(false), &result(is_error), None);
            assert!(!out.contains('\x1b'));
        }
    }
}
