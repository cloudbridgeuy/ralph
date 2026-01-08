//! Grep tool invocation formatting (verbose mode).
//!
//! Formats Grep tool invocations with detailed parameter display.

use ralph_core::stream::ToolInvocation;

use super::super::processor::StreamProcessor;

/// Format a Grep tool invocation with verbose output.
///
/// In verbose mode, the pattern is shown with regex syntax highlighting
/// and additional search parameters are displayed.
pub fn format_grep_tool_invocation_verbose(
    processor: &StreamProcessor,
    invocation: &ToolInvocation,
) -> String {
    // Extract the pattern from the input
    let pattern = invocation
        .input
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Extract optional search path
    let search_path = invocation
        .input
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");

    // Extract optional glob filter
    let glob = invocation.input.get("glob").and_then(|v| v.as_str());

    // Extract optional file type
    let file_type = invocation.input.get("type").and_then(|v| v.as_str());

    // Extract output mode
    let output_mode = invocation
        .input
        .get("output_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("files_with_matches");

    // Extract case-insensitive flag
    let case_insensitive = invocation
        .input
        .get("-i")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if processor.highlighting_enabled {
        let mut output = String::new();

        // Header with tool name
        output.push_str("\x1b[36m▶ Grep\x1b[0m\n");

        // Pattern line with regex highlighting
        output.push_str("  \x1b[1mPattern:\x1b[0m ");
        let highlighted_pattern = processor.code_highlighter.highlight(pattern, Some("regex"));
        // Remove trailing reset if present to add our own newline
        let trimmed_pattern = highlighted_pattern.trim_end_matches("\x1b[0m");
        output.push_str(trimmed_pattern);
        output.push_str("\x1b[0m\n");

        // Search path
        output.push_str(&format!("  \x1b[90mPath:\x1b[0m {}\n", search_path));

        // Output mode
        output.push_str(&format!("  \x1b[90mMode:\x1b[0m {}\n", output_mode));

        // Optional filters on same line if present
        let mut filters = Vec::new();
        if let Some(g) = glob {
            filters.push(format!("glob: {}", g));
        }
        if let Some(t) = file_type {
            filters.push(format!("type: {}", t));
        }
        if case_insensitive {
            filters.push("case-insensitive".to_string());
        }
        if !filters.is_empty() {
            output.push_str(&format!("  \x1b[90m[{}]\x1b[0m\n", filters.join(", ")));
        }

        output
    } else {
        // Plain text for non-terminal
        let mut output = String::new();

        output.push_str("> Grep\n");
        output.push_str(&format!("  Pattern: {}\n", pattern));
        output.push_str(&format!("  Path: {}\n", search_path));
        output.push_str(&format!("  Mode: {}\n", output_mode));

        // Optional filters
        let mut filters = Vec::new();
        if let Some(g) = glob {
            filters.push(format!("glob: {}", g));
        }
        if let Some(t) = file_type {
            filters.push(format!("type: {}", t));
        }
        if case_insensitive {
            filters.push("case-insensitive".to_string());
        }
        if !filters.is_empty() {
            output.push_str(&format!("  [{}]\n", filters.join(", ")));
        }

        output
    }
}
