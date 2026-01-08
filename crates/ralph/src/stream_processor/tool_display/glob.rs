//! Glob tool invocation formatting (verbose mode).
//!
//! Formats Glob tool invocations with detailed parameter display.

use ralph_core::stream::ToolInvocation;

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
    let search_path = invocation
        .input
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    if processor.highlighting_enabled {
        let mut output = String::new();

        // Header with tool name
        output.push_str("\x1b[36m▶ Glob\x1b[0m\n");

        // Pattern line - show full pattern without truncation
        output.push_str(&format!("  \x1b[1mPattern:\x1b[0m {}\n", pattern));

        // Search path
        output.push_str(&format!("  \x1b[90mPath:\x1b[0m {}\n", search_path));

        output
    } else {
        // Plain text for non-terminal
        let mut output = String::new();

        output.push_str("> Glob\n");
        output.push_str(&format!("  Pattern: {}\n", pattern));
        output.push_str(&format!("  Path: {}\n", search_path));

        output
    }
}
