//! Tool result display formatting.
//!
//! This module handles formatting tool execution results for terminal display.
//! Each tool type can have specialized result formatting, with a default fallback
//! for unknown tools.

mod bash;
mod edit;
mod grep;
mod read;
mod todowrite;

use ralph_core::chunk::is_unfenced_diff;
use ralph_core::stream::{ToolInvocation, ToolResult};

use super::processor::StreamProcessor;
use super::utils::truncate_string;

// Re-export for use in event_handler
pub use edit::format_edit_result_with_snapshot;

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
            format!("\x1b[31m✗ Error:\x1b[0m {}\n", truncated_content)
        } else {
            // Green check for success (dim output)
            format!("\x1b[32m✓\x1b[0m \x1b[90m{}\x1b[0m\n", truncated_content)
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
