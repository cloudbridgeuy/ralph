//! TodoWrite tool invocation formatting (verbose mode).
//!
//! Formats TodoWrite tool invocations with status indicators and colors.
//! Delegates to the shared render module for consistent formatting.

use ralph_core::stream::ToolInvocation;

use crate::render::{render_todowrite_invocation, TodoDisplayItem};

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
    let todos_json = invocation
        .input
        .get("todos")
        .and_then(|v| v.as_array())
        .map(|arr| arr.as_slice())
        .unwrap_or(&[]);

    // Convert to TodoDisplayItem array
    let todos: Vec<TodoDisplayItem> = todos_json
        .iter()
        .map(|todo| TodoDisplayItem {
            content: todo.get("content").and_then(|v| v.as_str()).unwrap_or(""),
            status: todo.get("status").and_then(|v| v.as_str()).unwrap_or(""),
            active_form: todo.get("activeForm").and_then(|v| v.as_str()),
        })
        .collect();

    render_todowrite_invocation(&processor.render_context(), &todos)
}
