//! Bash tool result formatting.
//!
//! Formats Bash tool execution results with output styling.

use ralph_core::stream::ToolResult;

use super::super::processor::StreamProcessor;
use super::super::utils::truncate_multiline;

/// Format a Bash tool result with distinct output styling.
///
/// The output is shown in a dimmed/muted color to distinguish it from the command.
/// Exit code is shown if non-zero (error indicator).
/// Very long outputs are truncated with a '... N more lines' indicator.
pub fn format_bash_tool_result(processor: &StreamProcessor, result: &ToolResult) -> String {
    const MAX_OUTPUT_LINES: usize = 30;

    if result.is_error {
        // Error case - show error message with red indicator
        let error_content = result
            .content
            .as_ref()
            .map(|c| truncate_multiline(c, MAX_OUTPUT_LINES))
            .unwrap_or_else(|| ("(command failed)".to_string(), false));

        if processor.highlighting_enabled {
            let mut output = String::new();
            output.push_str("\x1b[31m✗ Exit code: non-zero\x1b[0m\n");
            if !error_content.0.is_empty() {
                output.push_str("\x1b[90m");
                output.push_str(&error_content.0);
                output.push_str("\x1b[0m");
                if !error_content.0.ends_with('\n') {
                    output.push('\n');
                }
            }
            if error_content.1 {
                output.push_str("\x1b[90m... (output truncated)\x1b[0m\n");
            }
            output
        } else {
            let mut output = String::new();
            output.push_str("! Exit code: non-zero\n");
            if !error_content.0.is_empty() {
                output.push_str(&error_content.0);
                if !error_content.0.ends_with('\n') {
                    output.push('\n');
                }
            }
            if error_content.1 {
                output.push_str("... (output truncated)\n");
            }
            output
        }
    } else {
        // Success case - show output in dimmed style
        let content = result.content.as_deref().unwrap_or("");

        // Don't show anything for empty output
        if content.is_empty() {
            return if processor.highlighting_enabled {
                "\x1b[32m✓\x1b[0m\n".to_string()
            } else {
                "(ok)\n".to_string()
            };
        }

        let (display_content, truncated) = truncate_multiline(content, MAX_OUTPUT_LINES);

        if processor.highlighting_enabled {
            let mut output = String::new();
            output.push_str("\x1b[90m");
            output.push_str(&display_content);
            output.push_str("\x1b[0m");
            if !display_content.ends_with('\n') {
                output.push('\n');
            }
            if truncated {
                output.push_str("\x1b[90m... (output truncated)\x1b[0m\n");
            }
            output
        } else {
            let mut output = display_content.clone();
            if !output.ends_with('\n') {
                output.push('\n');
            }
            if truncated {
                output.push_str("... (output truncated)\n");
            }
            output
        }
    }
}
