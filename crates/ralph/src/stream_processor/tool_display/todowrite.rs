//! TodoWrite tool invocation formatting (verbose mode).
//!
//! Formats TodoWrite tool invocations with status indicators and colors.

use ralph_core::stream::ToolInvocation;

use super::super::processor::StreamProcessor;

/// Format a TodoWrite tool invocation with verbose output.
///
/// In verbose mode, the full todo list is displayed with status indicators
/// and color coding for each item's status.
pub fn format_todowrite_tool_invocation_verbose(
    processor: &StreamProcessor,
    invocation: &ToolInvocation,
) -> String {
    // Extract the todos array from the input
    let todos = invocation
        .input
        .get("todos")
        .and_then(|v| v.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or(&[]);

    if processor.highlighting_enabled {
        let mut output = String::new();

        // Header with tool name
        output.push_str("\x1b[36m▶ TodoWrite\x1b[0m\n");

        if todos.is_empty() {
            output.push_str("  \x1b[90m(clearing todo list)\x1b[0m\n");
        } else {
            // Display each todo item with status indicator
            for todo in todos {
                let content = todo.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let status = todo.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let active_form = todo.get("activeForm").and_then(|v| v.as_str());

                // Status indicator with color coding
                // ○ pending (default), ◐ in_progress (yellow), ● completed (green)
                let (icon, color) = match status {
                    "pending" => ("○", "\x1b[0m"),
                    "in_progress" => ("◐", "\x1b[33m"),
                    "completed" => ("●", "\x1b[32m"),
                    _ => ("?", "\x1b[90m"),
                };

                output.push_str(&format!("  {}{} {}\x1b[0m", color, icon, content));

                // Show activeForm if different from content
                if let Some(af) = active_form {
                    if af != content {
                        output.push_str(&format!(" \x1b[90m({})\x1b[0m", af));
                    }
                }

                output.push('\n');
            }
        }

        output
    } else {
        // Plain text for non-terminal
        let mut output = String::new();

        output.push_str("> TodoWrite\n");

        if todos.is_empty() {
            output.push_str("  (clearing todo list)\n");
        } else {
            for todo in todos {
                let content = todo.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let status = todo.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let active_form = todo.get("activeForm").and_then(|v| v.as_str());

                // Status indicator without color
                let icon = match status {
                    "pending" => "[ ]",
                    "in_progress" => "[~]",
                    "completed" => "[x]",
                    _ => "[?]",
                };

                output.push_str(&format!("  {} {}", icon, content));

                if let Some(af) = active_form {
                    if af != content {
                        output.push_str(&format!(" ({})", af));
                    }
                }

                output.push('\n');
            }
        }

        output
    }
}
