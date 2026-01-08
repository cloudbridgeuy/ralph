//! Edit tool result formatting.
//!
//! Formats Edit tool results with syntax-highlighted before/after display.
//!
//! Claude CLI's Edit tool returns success messages like "File updated", not actual
//! diff content. To show meaningful changes, we:
//! 1. Capture old_string and new_string from Edit tool input (via EditSnapshot)
//! 2. Display before block (old_string) with red background + syntax highlighting
//! 3. Display after block (new_string) with green background + syntax highlighting
//! 4. Fall back to unified diff if old_string/new_string unavailable

use std::fs;

use similar::{ChangeTag, TextDiff};

use crate::diff_highlight::highlight_with_basic_colors;
use ralph_core::stream::ToolInvocation;

use super::super::processor::StreamProcessor;
use super::super::types::EditSnapshot;
use super::super::utils::extract_language_from_path;

/// Maximum lines to show in a before/after block before truncating.
const MAX_BLOCK_LINES: usize = 30;

/// Maximum lines to show in a unified diff before truncating.
const MAX_DIFF_LINES: usize = 50;

/// ANSI color codes for background highlighting.
/// Vibrant red background (24-bit RGB: 100, 40, 40) - distinguishable deletion indicator
const RED_BG: &str = "\x1b[48;2;100;40;40m";
/// Vibrant green background (24-bit RGB: 40, 90, 40) - distinguishable addition indicator
const GREEN_BG: &str = "\x1b[48;2;40;90;40m";
/// Reset all attributes
const RESET: &str = "\x1b[0m";
/// Dim gray foreground
const DIM: &str = "\x1b[90m";
/// Cyan foreground
const CYAN: &str = "\x1b[36m";
/// Green foreground (for checkmark)
const GREEN: &str = "\x1b[32m";

/// Format an Edit tool result using before/after display from snapshot.
///
/// This is called when we have captured a snapshot before the edit.
/// It uses old_string/new_string for before/after display, falling back
/// to unified diff if those are unavailable.
pub fn format_edit_result_with_snapshot(
    processor: &StreamProcessor,
    snapshot: EditSnapshot,
) -> String {
    // Prefer before/after display if old_string and new_string are available
    if snapshot.old_string.is_some() || snapshot.new_string.is_some() {
        return format_before_after_display(processor, &snapshot);
    }

    // Fall back to unified diff from file content comparison
    if let Some(diff_content) = generate_diff_from_snapshot(&snapshot) {
        format_diff_output(processor, &snapshot.file_path, &diff_content)
    } else {
        // No changes or couldn't read file - show simple success message
        format_no_changes_message(processor, &snapshot.file_path)
    }
}

/// Format the before/after display with syntax highlighting and colored backgrounds.
fn format_before_after_display(processor: &StreamProcessor, snapshot: &EditSnapshot) -> String {
    let old_str = snapshot.old_string.as_deref().unwrap_or("");
    let new_str = snapshot.new_string.as_deref().unwrap_or("");

    // Detect language from file path
    let language = extract_language_from_path(&snapshot.file_path);

    if processor.highlighting_enabled {
        format_before_after_highlighted(processor, &snapshot.file_path, old_str, new_str, language)
    } else {
        format_before_after_plain(&snapshot.file_path, old_str, new_str)
    }
}

/// Format before/after blocks with syntax highlighting and colored backgrounds.
fn format_before_after_highlighted(
    processor: &StreamProcessor,
    file_path: &str,
    old_str: &str,
    new_str: &str,
    language: Option<&'static str>,
) -> String {
    let mut output = String::new();

    // File path header with box drawing
    output.push_str(&format!("{}── {} ──{}\n", CYAN, file_path, RESET));

    // Before block
    output.push_str(&format_highlighted_block(
        processor, "Before:", old_str, language, RED_BG,
    ));

    // Horizontal separator
    output.push_str(&format!("{}─────────────────────────{}\n", DIM, RESET));

    // After block
    output.push_str(&format_highlighted_block(
        processor, "After:", new_str, language, GREEN_BG,
    ));

    output
}

/// Format a single highlighted block (before or after) with background color.
fn format_highlighted_block(
    processor: &StreamProcessor,
    label: &str,
    content: &str,
    language: Option<&'static str>,
    bg_color: &str,
) -> String {
    let mut output = String::new();

    // Label
    output.push_str(&format!("{}{}{}\n", DIM, label, RESET));

    // Handle empty content
    if content.is_empty() {
        output.push_str(&format!("{}(empty){}\n", DIM, RESET));
        return output;
    }

    // Count and potentially truncate lines
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let (display_lines, truncated) = if line_count > MAX_BLOCK_LINES {
        (&lines[..MAX_BLOCK_LINES], true)
    } else {
        (&lines[..], false)
    };

    let display_content = display_lines.join("\n");

    // Apply syntax highlighting first (if language is detected)
    let highlighted = if let Some(lang) = language {
        processor
            .code_highlighter
            .highlight(&display_content, Some(lang))
    } else {
        display_content.clone()
    };

    // Apply background color to line numbers only, syntax highlighting to content
    let line_count_width = display_lines.len().to_string().len().max(2);
    for (i, line) in highlighted.lines().enumerate() {
        let line_num = i + 1;
        // Format: line number with background │ content with syntax highlighting only
        output.push_str(&format!(
            "{}{:>width$} {}│{} {}\n",
            bg_color,
            line_num,
            RESET,
            RESET,
            line,
            width = line_count_width
        ));
    }

    // Truncation indicator
    if truncated {
        output.push_str(&format!(
            "{}... {} more lines{}\n",
            DIM,
            line_count - MAX_BLOCK_LINES,
            RESET
        ));
    }

    output
}

/// Format before/after blocks in plain text (no ANSI codes).
fn format_before_after_plain(file_path: &str, old_str: &str, new_str: &str) -> String {
    let mut output = String::new();

    // File path header
    output.push_str(&format!("-- {} --\n", file_path));

    // Before block
    output.push_str(&format_plain_block("Before:", old_str));

    // Horizontal separator
    output.push_str("-------------------------\n");

    // After block
    output.push_str(&format_plain_block("After:", new_str));

    output
}

/// Format a single plain text block (before or after).
fn format_plain_block(label: &str, content: &str) -> String {
    let mut output = String::new();

    output.push_str(&format!("{}\n", label));

    if content.is_empty() {
        output.push_str("(empty)\n");
        return output;
    }

    // Count and potentially truncate lines
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let (display_lines, truncated) = if line_count > MAX_BLOCK_LINES {
        (&lines[..MAX_BLOCK_LINES], true)
    } else {
        (&lines[..], false)
    };

    let line_count_width = display_lines.len().to_string().len().max(2);
    for (i, line) in display_lines.iter().enumerate() {
        let line_num = i + 1;
        output.push_str(&format!(
            "{:>width$} │ {}\n",
            line_num,
            line,
            width = line_count_width
        ));
    }

    if truncated {
        output.push_str(&format!(
            "... {} more lines\n",
            line_count - MAX_BLOCK_LINES
        ));
    }

    output
}

/// Format a simple "no changes" message.
fn format_no_changes_message(processor: &StreamProcessor, file_path: &str) -> String {
    if processor.highlighting_enabled {
        format!(
            "{}✓{} {}Edit: {} (no changes){}\n",
            GREEN, RESET, DIM, file_path, RESET
        )
    } else {
        format!("  Edit: {} (no changes)\n", file_path)
    }
}

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
        output.push_str(&format!("{}── {} ──{}\n", CYAN, file_path, RESET));

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
                "{}... {} more lines{}\n",
                DIM,
                line_count - MAX_DIFF_LINES,
                RESET
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

    #[test]
    fn test_format_before_after_plain() {
        let output = format_before_after_plain("test.rs", "old code", "new code");

        assert!(output.contains("-- test.rs --"));
        assert!(output.contains("Before:"));
        assert!(output.contains("old code"));
        assert!(output.contains("After:"));
        assert!(output.contains("new code"));
    }

    #[test]
    fn test_format_before_after_plain_empty_old() {
        let output = format_before_after_plain("test.rs", "", "new code");

        assert!(output.contains("(empty)"));
        assert!(output.contains("new code"));
    }

    #[test]
    fn test_format_before_after_plain_empty_new() {
        let output = format_before_after_plain("test.rs", "old code", "");

        assert!(output.contains("old code"));
        assert!(output.contains("(empty)"));
    }

    #[test]
    fn test_format_plain_block_truncation() {
        let long_content: String = (0..50)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");

        let output = format_plain_block("Test:", &long_content);

        assert!(output.contains("... 20 more lines"));
    }

    #[test]
    fn test_format_plain_block_no_truncation() {
        let short_content = "line1\nline2\nline3";

        let output = format_plain_block("Test:", short_content);

        assert!(!output.contains("more lines"));
        assert!(output.contains("line1"));
        assert!(output.contains("line3"));
    }

    #[test]
    fn test_format_before_after_highlighted_has_ansi_codes() {
        let processor = StreamProcessor::with_highlighting(true);

        let output = format_before_after_highlighted(
            &processor,
            "test.rs",
            "let x = 1;",
            "let x = 2;",
            Some("rust"),
        );

        // Should contain ANSI escape codes
        assert!(output.contains("\x1b["));
        // Should contain the file path
        assert!(output.contains("test.rs"));
        // Should contain "Before:" and "After:" labels
        assert!(output.contains("Before:"));
        assert!(output.contains("After:"));
        // Should contain the background color codes
        assert!(output.contains(RED_BG) || output.contains(GREEN_BG));
    }

    #[test]
    fn test_format_before_after_display_prefers_old_new_strings() {
        let processor = StreamProcessor::with_highlighting(false);
        let snapshot = EditSnapshot {
            file_path: "test.rs".to_string(),
            content: Some("full file content".to_string()),
            old_string: Some("old".to_string()),
            new_string: Some("new".to_string()),
        };

        let output = format_edit_result_with_snapshot(&processor, snapshot);

        // Should use old_string/new_string, not full file diff
        assert!(output.contains("Before:"));
        assert!(output.contains("old"));
        assert!(output.contains("After:"));
        assert!(output.contains("new"));
    }

    #[test]
    fn test_format_no_changes_message_highlighted() {
        let processor = StreamProcessor::with_highlighting(true);

        let output = format_no_changes_message(&processor, "test.rs");

        assert!(output.contains("✓"));
        assert!(output.contains("test.rs"));
        assert!(output.contains("no changes"));
    }

    #[test]
    fn test_format_no_changes_message_plain() {
        let processor = StreamProcessor::with_highlighting(false);

        let output = format_no_changes_message(&processor, "test.rs");

        assert!(output.contains("test.rs"));
        assert!(output.contains("no changes"));
        // Should not contain ANSI codes
        assert!(!output.contains("\x1b["));
    }
}
