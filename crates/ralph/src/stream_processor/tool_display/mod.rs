//! Tool invocation display formatting.
//!
//! This module handles formatting tool invocations for terminal display.
//! Each tool type can have specialized formatting, with a default fallback
//! for unknown tools.

mod bash;
mod glob;
mod grep;
mod read;
mod todowrite;

use ralph_core::stream::ToolInvocation;

use super::processor::StreamProcessor;
use super::utils::{extract_key_argument, truncate_string};

/// Format a tool invocation for display.
///
/// File paths are shown in full without truncation for tools like Read, Edit,
/// Write, Glob, and Grep. Other arguments (like Bash commands or prompts)
/// are truncated to keep output readable.
///
/// Bash tool invocations receive special treatment: the command is shown in
/// full with shell syntax highlighting applied.
///
/// Grep tool invocations receive special treatment in verbose mode: the
/// regex pattern is shown with syntax highlighting.
pub fn format_tool_invocation(processor: &StreamProcessor, invocation: &ToolInvocation) -> String {
    // Special handling for Bash tool invocations
    if invocation.name == "Bash" {
        return bash::format_bash_tool_invocation(processor, invocation);
    }

    // Special handling for Grep tool invocations in verbose mode
    if invocation.name == "Grep" && processor.is_tool_verbose("Grep") {
        return grep::format_grep_tool_invocation_verbose(processor, invocation);
    }

    // Special handling for Read tool invocations in verbose mode
    if invocation.name == "Read" && processor.is_tool_verbose("Read") {
        return read::format_read_tool_invocation_verbose(processor, invocation);
    }

    // Special handling for TodoWrite tool invocations in verbose mode
    if invocation.name == "TodoWrite" && processor.is_tool_verbose("TodoWrite") {
        return todowrite::format_todowrite_tool_invocation_verbose(processor, invocation);
    }

    // Special handling for Glob tool invocations in verbose mode
    if invocation.name == "Glob" && processor.is_tool_verbose("Glob") {
        return glob::format_glob_tool_invocation_verbose(processor, invocation);
    }

    let key_arg = extract_key_argument(&invocation.name, &invocation.input);

    // Format the argument: paths shown in full, other args truncated
    let formatted_arg = key_arg.map(|arg| {
        if arg.is_path {
            arg.value
        } else {
            truncate_string(&arg.value, 60)
        }
    });

    if processor.highlighting_enabled {
        // Use colors for terminal display
        format!(
            "\x1b[36m▶ {}\x1b[0m{}\n",
            invocation.name,
            if let Some(arg) = formatted_arg {
                format!(" \x1b[90m{}\x1b[0m", arg)
            } else {
                String::new()
            }
        )
    } else {
        // Plain text for non-terminal
        format!(
            "> {} {}\n",
            invocation.name,
            formatted_arg.unwrap_or_default()
        )
    }
}
