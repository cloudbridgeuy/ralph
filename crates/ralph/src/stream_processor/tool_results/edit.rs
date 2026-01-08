//! Edit tool result formatting.
//!
//! Formats Edit tool results with generated diffs and syntax highlighting.
//!
//! Claude CLI's Edit tool returns success messages like "File updated", not actual
//! diff content. To show meaningful diffs, we:
//! 1. Capture file content before the Edit tool executes (via EditSnapshot)
//! 2. Read the file again after the edit completes
//! 3. Generate a unified diff between before and after
//! 4. Display with syntax highlighting

use std::fs;

use similar::{ChangeTag, TextDiff};

use crate::diff_highlight::highlight_with_basic_colors;
use ralph_core::stream::ToolInvocation;

use super::super::processor::StreamProcessor;
use super::super::types::EditSnapshot;

/// Maximum lines to show in a diff before truncating.
const MAX_DIFF_LINES: usize = 50;

/// Generate a unified diff from an EditSnapshot by comparing before/after content.
///
/// Reads the current file content and generates a unified diff against the
/// captured snapshot. Returns None if the file cannot be read or no changes
/// were made.
pub fn generate_diff_from_snapshot(snapshot: &EditSnapshot) -> Option<String> {
    // Read current file content
    let after_content = fs::read_to_string(&snapshot.file_path).ok()?;

    // Get before content (empty string if file didn't exist)
    let before_content = snapshot.content.as_deref().unwrap_or("");

    // Skip if no changes
    if before_content == after_content {
        return None;
    }

    // Generate unified diff
    let diff = TextDiff::from_lines(before_content, &after_content);
    let unified = generate_unified_diff(&diff, &snapshot.file_path);

    Some(unified)
}

/// Generate unified diff format from a TextDiff.
fn generate_unified_diff<'a>(diff: &TextDiff<'a, 'a, 'a, str>, file_path: &str) -> String {
    let mut output = String::new();

    // Add header
    output.push_str(&format!("--- a/{}\n", file_path));
    output.push_str(&format!("+++ b/{}\n", file_path));

    // Generate hunks
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        // Hunk header
        output.push_str(&format!("{}", hunk.header()));

        // Changes
        for change in hunk.iter_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            output.push_str(sign);
            output.push_str(change.value());
            if !change.value().ends_with('\n') {
                output.push('\n');
            }
        }
    }

    output
}

/// Format an Edit tool result using a generated diff from snapshot.
///
/// This is called when we have captured a snapshot before the edit.
/// It reads the current file content, generates a diff, and formats it
/// with syntax highlighting.
pub fn format_edit_result_with_snapshot(
    processor: &StreamProcessor,
    snapshot: EditSnapshot,
) -> String {
    // Try to generate diff from snapshot
    if let Some(diff_content) = generate_diff_from_snapshot(&snapshot) {
        format_diff_output(processor, &snapshot.file_path, &diff_content)
    } else {
        // No changes or couldn't read file - show simple success message
        if processor.highlighting_enabled {
            format!(
                "\x1b[32m✓\x1b[0m \x1b[90mEdit: {} (no changes)\x1b[0m\n",
                snapshot.file_path
            )
        } else {
            format!("  Edit: {} (no changes)\n", snapshot.file_path)
        }
    }
}

/// Format an Edit tool result that already contains diff content.
///
/// This is the fallback path used when the Edit tool result already contains
/// diff content (e.g., from older versions or custom tools).
pub fn format_edit_diff_result(
    processor: &StreamProcessor,
    invocation: ToolInvocation,
    diff_content: &str,
) -> String {
    // Extract file path from the invocation input
    let file_path = invocation
        .input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown file");

    format_diff_output(processor, file_path, diff_content)
}

/// Format diff output with optional highlighting and truncation.
///
/// This is the common formatting logic shared between generated diffs
/// and diffs that come directly from tool results.
fn format_diff_output(processor: &StreamProcessor, file_path: &str, diff_content: &str) -> String {
    // Count lines for potential truncation
    let lines: Vec<&str> = diff_content.lines().collect();
    let line_count = lines.len();

    // Truncate if too long
    let (display_content, truncated) = if line_count > MAX_DIFF_LINES {
        let truncated_lines: String = lines[..MAX_DIFF_LINES].join("\n");
        (truncated_lines, true)
    } else {
        (diff_content.to_string(), false)
    };

    if processor.highlighting_enabled {
        // Highlight the diff
        let highlighted_diff = highlight_with_basic_colors(&display_content);

        // Build output with header
        let mut output = String::new();

        // File path header with box drawing
        output.push_str(&format!("\x1b[36m── {} ──\x1b[0m\n", file_path));

        // The highlighted diff content wrapped in diff fences
        output.push_str("```diff\n");
        output.push_str(&highlighted_diff);
        if !highlighted_diff.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("```\n");

        // Truncation indicator
        if truncated {
            output.push_str(&format!(
                "\x1b[90m... {} more lines\x1b[0m\n",
                line_count - MAX_DIFF_LINES
            ));
        }

        output
    } else {
        // Plain text format
        let mut output = String::new();

        // Simple header
        output.push_str(&format!("-- {} --\n", file_path));

        // Plain diff content
        output.push_str("```diff\n");
        output.push_str(&display_content);
        if !display_content.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("```\n");

        // Truncation indicator
        if truncated {
            output.push_str(&format!("... {} more lines\n", line_count - MAX_DIFF_LINES));
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_unified_diff() {
        let before = "line1\nline2\nline3\n";
        let after = "line1\nmodified\nline3\n";

        let diff = TextDiff::from_lines(before, after);
        let unified = generate_unified_diff(&diff, "test.txt");

        assert!(unified.contains("--- a/test.txt"));
        assert!(unified.contains("+++ b/test.txt"));
        assert!(unified.contains("-line2"));
        assert!(unified.contains("+modified"));
    }

    #[test]
    fn test_generate_unified_diff_addition() {
        let before = "line1\n";
        let after = "line1\nline2\n";

        let diff = TextDiff::from_lines(before, after);
        let unified = generate_unified_diff(&diff, "test.txt");

        assert!(unified.contains("+line2"));
        assert!(!unified.contains("-line1")); // line1 unchanged
    }

    #[test]
    fn test_generate_unified_diff_deletion() {
        let before = "line1\nline2\n";
        let after = "line1\n";

        let diff = TextDiff::from_lines(before, after);
        let unified = generate_unified_diff(&diff, "test.txt");

        assert!(unified.contains("-line2"));
        assert!(!unified.contains("+line2"));
    }

    #[test]
    fn test_format_diff_output_truncation() {
        let processor = StreamProcessor::with_highlighting(false);
        let long_diff: String = (0..100).map(|i| format!("+line{}\n", i)).collect();

        let output = format_diff_output(&processor, "test.txt", &long_diff);

        assert!(output.contains("... 50 more lines"));
    }

    #[test]
    fn test_format_diff_output_no_truncation() {
        let processor = StreamProcessor::with_highlighting(false);
        let short_diff = "+line1\n+line2\n";

        let output = format_diff_output(&processor, "test.txt", short_diff);

        assert!(!output.contains("more lines"));
    }
}
