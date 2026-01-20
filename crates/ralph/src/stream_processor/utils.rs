//! Pure utility functions for the stream processor (Functional Core).
//!
//! This module contains stateless helper functions for argument extraction,
//! string truncation, and language detection. All functions are pure with
//! no side effects.

use super::types::KeyArgument;
use serde_json::Value;

// Re-export language detection from shared render module for backward compatibility
pub use crate::render::extract_language_from_path;

/// Extract the most relevant argument from a tool invocation for display.
///
/// Different tools have different key arguments:
/// - Read/Edit/Write: file_path (shown in full)
/// - Glob: pattern (shown in full)
/// - Grep: pattern (shown in full)
/// - Bash: command (truncated)
/// - WebFetch: url (shown in full)
/// - Task: prompt (truncated)
pub fn extract_key_argument(tool_name: &str, input: &Value) -> Option<KeyArgument> {
    let obj = input.as_object()?;

    // Tool-specific key arguments with path indicators
    let (key, is_path) = match tool_name {
        "Read" | "Edit" | "Write" => ("file_path", true),
        "Glob" | "Grep" => ("pattern", true),
        "Bash" => ("command", false),
        "WebFetch" => ("url", true),
        "Task" => ("prompt", false),
        "NotebookEdit" => ("notebook_path", true),
        _ => {
            // For unknown tools, try common field names
            if obj.contains_key("file_path") {
                ("file_path", true)
            } else if obj.contains_key("path") {
                ("path", true)
            } else if obj.contains_key("pattern") {
                ("pattern", true)
            } else if obj.contains_key("command") {
                ("command", false)
            } else {
                // Return the first string value (truncated since we don't know what it is)
                for (_, v) in obj {
                    if let Some(s) = v.as_str() {
                        return Some(KeyArgument {
                            value: s.to_string(),
                            is_path: false,
                        });
                    }
                }
                return None;
            }
        }
    };

    obj.get(key).and_then(|v| v.as_str()).map(|s| KeyArgument {
        value: s.to_string(),
        is_path,
    })
}

/// Truncate a string to a maximum length, adding ellipsis if needed.
pub fn truncate_string(s: &str, max_len: usize) -> String {
    // First, replace newlines with spaces for cleaner display
    let single_line: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();

    // Then truncate if needed
    if single_line.len() <= max_len {
        single_line
    } else {
        let truncated: String = single_line.chars().take(max_len - 3).collect();
        format!("{}...", truncated)
    }
}

/// Detect if content was truncated by Claude.
///
/// Claude adds truncation indicators like "... (truncated)" or "Output truncated"
/// when output exceeds limits.
pub fn is_content_truncated(content: &str) -> bool {
    content.contains("... (truncated)") || content.contains("Output truncated")
}

/// Count non-empty lines in content.
///
/// Useful for calculating match counts in Grep results or file counts in Glob results.
pub fn count_non_empty_lines(content: &str) -> usize {
    content.lines().filter(|l| !l.is_empty()).count()
}

/// Truncate multiline content to a maximum number of lines.
///
/// Returns a tuple of (truncated content, was_truncated).
pub fn truncate_multiline(s: &str, max_lines: usize) -> (String, bool) {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= max_lines {
        (s.to_string(), false)
    } else {
        let truncated = lines[..max_lines].join("\n");
        (truncated, true)
    }
}

// Tests for extract_language_from_path are in crate::render::utils
