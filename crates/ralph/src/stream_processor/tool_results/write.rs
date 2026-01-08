//! Write tool result formatting.
//!
//! Formats Write tool results with generated diffs and syntax highlighting.
//!
//! Claude CLI's Write tool creates or overwrites files, returning success messages.
//! To show meaningful diffs, we:
//! 1. Capture file content before the Write tool executes (via WriteSnapshot)
//! 2. Read the file again after the write completes
//! 3. Generate a unified diff between before and after (or show all additions for new files)
//! 4. Display with syntax highlighting

use std::fs;

use similar::{ChangeTag, TextDiff};

use crate::diff_highlight::highlight_with_basic_colors;

use super::super::processor::StreamProcessor;
use super::super::types::WriteSnapshot;

/// Maximum lines to show in a diff before truncating.
const MAX_DIFF_LINES: usize = 50;

/// Generate a unified diff from a WriteSnapshot by comparing before/after content.
///
/// Reads the current file content and generates a unified diff against the
/// captured snapshot. Returns None if the file cannot be read or no changes
/// were made.
pub fn generate_diff_from_snapshot(snapshot: &WriteSnapshot) -> Option<String> {
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

/// Format a Write tool result using a generated diff from snapshot.
///
/// This is called when we have captured a snapshot before the write.
/// It reads the current file content, generates a diff, and formats it
/// with syntax highlighting.
pub fn format_write_result_with_snapshot(
    processor: &StreamProcessor,
    snapshot: WriteSnapshot,
) -> String {
    // Try to generate diff from snapshot
    if let Some(diff_content) = generate_diff_from_snapshot(&snapshot) {
        format_diff_output(
            processor,
            &snapshot.file_path,
            &diff_content,
            !snapshot.file_existed,
        )
    } else {
        // No changes or couldn't read file - show simple success message
        if processor.highlighting_enabled {
            format!(
                "\x1b[32m\u{2713}\x1b[0m \x1b[90mWrite: {} (no changes)\x1b[0m\n",
                snapshot.file_path
            )
        } else {
            format!("  Write: {} (no changes)\n", snapshot.file_path)
        }
    }
}

/// Format diff output with optional highlighting and truncation.
///
/// This is the common formatting logic for Write tool diffs.
/// The `is_new_file` flag indicates whether the file was newly created.
fn format_diff_output(
    processor: &StreamProcessor,
    file_path: &str,
    diff_content: &str,
    is_new_file: bool,
) -> String {
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

    // Build file indicator
    let file_indicator = if is_new_file { "(new file)" } else { "" };

    if processor.highlighting_enabled {
        // Highlight the diff
        let highlighted_diff = highlight_with_basic_colors(&display_content);

        // Build output with header
        let mut output = String::new();

        // File path header with box drawing and new file indicator
        if is_new_file {
            output.push_str(&format!(
                "\x1b[36m\u{2500}\u{2500} {} \x1b[33m{}\x1b[36m \u{2500}\u{2500}\x1b[0m\n",
                file_path, file_indicator
            ));
        } else {
            output.push_str(&format!(
                "\x1b[36m\u{2500}\u{2500} {} \u{2500}\u{2500}\x1b[0m\n",
                file_path
            ));
        }

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

        // Simple header with new file indicator
        if is_new_file {
            output.push_str(&format!("-- {} {} --\n", file_path, file_indicator));
        } else {
            output.push_str(&format!("-- {} --\n", file_path));
        }

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
    fn test_generate_unified_diff_new_file() {
        let before = "";
        let after = "line1\nline2\n";

        let diff = TextDiff::from_lines(before, after);
        let unified = generate_unified_diff(&diff, "new_file.txt");

        assert!(unified.contains("+line1"));
        assert!(unified.contains("+line2"));
        assert!(!unified.contains("-line1"));
    }

    #[test]
    fn test_format_diff_output_new_file_indicator() {
        let processor = StreamProcessor::with_highlighting(false);
        let diff = "+line1\n+line2\n";

        let output = format_diff_output(&processor, "new.txt", diff, true);

        assert!(output.contains("(new file)"));
    }

    #[test]
    fn test_format_diff_output_overwrite_no_indicator() {
        let processor = StreamProcessor::with_highlighting(false);
        let diff = "-old\n+new\n";

        let output = format_diff_output(&processor, "existing.txt", diff, false);

        assert!(!output.contains("(new file)"));
    }

    #[test]
    fn test_format_diff_output_truncation() {
        let processor = StreamProcessor::with_highlighting(false);
        let long_diff: String = (0..100).map(|i| format!("+line{}\n", i)).collect();

        let output = format_diff_output(&processor, "test.txt", &long_diff, false);

        assert!(output.contains("... 50 more lines"));
    }
}
