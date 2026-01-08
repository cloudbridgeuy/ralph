//! Read tool invocation formatting (verbose mode).
//!
//! Formats Read tool invocations with file path and range information.

use ralph_core::stream::ToolInvocation;

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

    if processor.highlighting_enabled {
        let mut output = String::new();

        // Header with tool name
        output.push_str("\x1b[36m▶ Read\x1b[0m\n");

        // File path
        output.push_str(&format!("  \x1b[1mFile:\x1b[0m {}\n", file_path));

        // Optional range info
        let mut range_parts = Vec::new();
        if let Some(o) = offset {
            range_parts.push(format!("offset: {}", o));
        }
        if let Some(l) = limit {
            range_parts.push(format!("limit: {}", l));
        }
        if !range_parts.is_empty() {
            output.push_str(&format!("  \x1b[90m[{}]\x1b[0m\n", range_parts.join(", ")));
        }

        output
    } else {
        // Plain text for non-terminal
        let mut output = String::new();

        output.push_str("> Read\n");
        output.push_str(&format!("  File: {}\n", file_path));

        // Optional range info
        let mut range_parts = Vec::new();
        if let Some(o) = offset {
            range_parts.push(format!("offset: {}", o));
        }
        if let Some(l) = limit {
            range_parts.push(format!("limit: {}", l));
        }
        if !range_parts.is_empty() {
            output.push_str(&format!("  [{}]\n", range_parts.join(", ")));
        }

        output
    }
}
