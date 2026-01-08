//! Bash tool invocation formatting.
//!
//! Formats Bash tool invocations with syntax highlighting.

use ralph_core::stream::ToolInvocation;

use super::super::processor::StreamProcessor;

/// Format a Bash tool invocation with syntax highlighting.
///
/// The command is shown in full (not truncated) with shell syntax highlighting
/// applied. Multi-line commands are displayed with proper formatting.
pub fn format_bash_tool_invocation(
    processor: &StreamProcessor,
    invocation: &ToolInvocation,
) -> String {
    // Extract the command from the input
    let command = invocation
        .input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Check if this is a multi-line command
    let is_multiline = command.contains('\n');

    if processor.highlighting_enabled {
        let mut output = String::new();

        // Header with tool name
        output.push_str("\x1b[36m▶ Bash\x1b[0m\n");

        if is_multiline {
            // Multi-line: wrap in a code block with shell highlighting
            output.push_str("```sh\n");
            let highlighted = processor.code_highlighter.highlight(command, Some("sh"));
            output.push_str(&highlighted);
            if !highlighted.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("```\n");
        } else {
            // Single-line: show inline with highlighting
            output.push_str("  ");
            let highlighted = processor.code_highlighter.highlight(command, Some("sh"));
            // Remove trailing reset if present to add our own formatting
            let trimmed = highlighted.trim_end_matches("\x1b[0m");
            output.push_str(trimmed);
            output.push_str("\x1b[0m\n");
        }

        output
    } else {
        // Plain text for non-terminal
        if is_multiline {
            let mut output = String::new();
            output.push_str("> Bash\n");
            output.push_str("```sh\n");
            output.push_str(command);
            if !command.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("```\n");
            output
        } else {
            format!("> Bash\n  {}\n", command)
        }
    }
}
