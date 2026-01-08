//! Write tool result formatting.
//!
//! Formats Write tool results with generated diffs and syntax highlighting.
//!
//! Claude CLI's Write tool creates or overwrites files, returning success messages.
//! To show meaningful diffs, we:
//! 1. Capture file content before the Write tool executes (via WriteSnapshot)
//! 2. Read the file again after the write completes
//! 3. For new files: display content with syntax highlighting and green line numbers
//! 4. For overwrites: generate a unified diff between before and after
//! 5. Display with syntax highlighting

use std::fs;

use similar::{ChangeTag, TextDiff};

use crate::diff_highlight::highlight_with_basic_colors;

use super::super::processor::StreamProcessor;
use super::super::types::WriteSnapshot;
use super::super::utils::extract_language_from_path;

/// Maximum lines to show in a new file display before truncating.
const MAX_NEW_FILE_LINES: usize = 50;

/// Maximum lines to show in a diff before truncating.
const MAX_DIFF_LINES: usize = 50;

/// ANSI color codes for background highlighting.
/// Vibrant green background (24-bit RGB: 40, 90, 40) - distinguishable addition indicator
const GREEN_BG: &str = "\x1b[48;2;40;90;40m";
/// Reset all attributes
const RESET: &str = "\x1b[0m";
/// Dim gray foreground
const DIM: &str = "\x1b[90m";
/// Cyan foreground
const CYAN: &str = "\x1b[36m";
/// Yellow foreground (for indicators)
const YELLOW: &str = "\x1b[33m";
/// Green foreground (for checkmark)
const GREEN: &str = "\x1b[32m";

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
/// For new files, displays clean syntax-highlighted content with green line numbers.
/// For overwrites, generates and displays a unified diff.
pub fn format_write_result_with_snapshot(
    processor: &StreamProcessor,
    snapshot: WriteSnapshot,
) -> String {
    // Handle new files: show clean syntax-highlighted content without diff markers
    if !snapshot.file_existed {
        // Read the newly created file content
        if let Ok(content) = fs::read_to_string(&snapshot.file_path) {
            if !content.is_empty() {
                return format_new_file_display(processor, &snapshot.file_path, &content);
            }
        }
        // Empty new file or couldn't read
        return format_no_changes_message(processor, &snapshot.file_path, true);
    }

    // Handle overwrites: generate and display unified diff
    if let Some(diff_content) = generate_diff_from_snapshot(&snapshot) {
        format_diff_output(processor, &snapshot.file_path, &diff_content)
    } else {
        // No changes or couldn't read file - show simple success message
        format_no_changes_message(processor, &snapshot.file_path, false)
    }
}

/// Format a "no changes" or success message.
fn format_no_changes_message(
    processor: &StreamProcessor,
    file_path: &str,
    is_new_file: bool,
) -> String {
    let indicator = if is_new_file {
        "(new file)"
    } else {
        "(no changes)"
    };
    if processor.highlighting_enabled {
        format!(
            "{}✓{} {}Write: {} {}{}\n",
            GREEN, RESET, DIM, file_path, indicator, RESET
        )
    } else {
        format!("  Write: {} {}\n", file_path, indicator)
    }
}

/// Format new file content with syntax highlighting and green line number backgrounds.
///
/// Displays the file content without diff markers (+/-), using the same style as
/// Edit tool's "After" block: green background on line numbers, syntax highlighting
/// on content.
fn format_new_file_display(processor: &StreamProcessor, file_path: &str, content: &str) -> String {
    // Detect language from file path
    let language = extract_language_from_path(file_path);

    if processor.highlighting_enabled {
        format_new_file_highlighted(processor, file_path, content, language)
    } else {
        format_new_file_plain(file_path, content)
    }
}

/// Format new file content with syntax highlighting and colored line numbers.
fn format_new_file_highlighted(
    processor: &StreamProcessor,
    file_path: &str,
    content: &str,
    language: Option<&'static str>,
) -> String {
    let mut output = String::new();

    // File path header with box drawing and (new file) indicator
    output.push_str(&format!(
        "{}── {} {}(new file){} {}──{}\n",
        CYAN, file_path, YELLOW, RESET, CYAN, RESET
    ));

    // Count and potentially truncate lines
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let (display_lines, truncated) = if line_count > MAX_NEW_FILE_LINES {
        (&lines[..MAX_NEW_FILE_LINES], true)
    } else {
        (&lines[..], false)
    };

    let display_content = display_lines.join("\n");

    // Apply syntax highlighting (if language is detected)
    let highlighted = if let Some(lang) = language {
        processor
            .code_highlighter
            .highlight(&display_content, Some(lang))
    } else {
        display_content.clone()
    };

    // Apply green background color to line numbers only
    let line_count_width = display_lines.len().to_string().len().max(2);
    for (i, line) in highlighted.lines().enumerate() {
        let line_num = i + 1;
        output.push_str(&format!(
            "{}{:>width$} {}│{} {}\n",
            GREEN_BG,
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
            line_count - MAX_NEW_FILE_LINES,
            RESET
        ));
    }

    output
}

/// Format new file content in plain text (no ANSI codes).
fn format_new_file_plain(file_path: &str, content: &str) -> String {
    let mut output = String::new();

    // File path header with (new file) indicator
    output.push_str(&format!("-- {} (new file) --\n", file_path));

    // Count and potentially truncate lines
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let (display_lines, truncated) = if line_count > MAX_NEW_FILE_LINES {
        (&lines[..MAX_NEW_FILE_LINES], true)
    } else {
        (&lines[..], false)
    };

    // Display lines with line numbers
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

    // Truncation indicator
    if truncated {
        output.push_str(&format!(
            "... {} more lines\n",
            line_count - MAX_NEW_FILE_LINES
        ));
    }

    output
}

/// Format diff output with optional highlighting and truncation.
///
/// This is used for displaying unified diffs when an existing file is overwritten.
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
    fn test_format_diff_output_overwrite_no_indicator() {
        let processor = StreamProcessor::with_highlighting(false);
        let diff = "-old\n+new\n";

        let output = format_diff_output(&processor, "existing.txt", diff);

        // Overwrites should not show "(new file)" indicator
        assert!(!output.contains("(new file)"));
        // Should have file path header
        assert!(output.contains("existing.txt"));
    }

    #[test]
    fn test_format_diff_output_truncation() {
        let processor = StreamProcessor::with_highlighting(false);
        let long_diff: String = (0..100).map(|i| format!("+line{}\n", i)).collect();

        let output = format_diff_output(&processor, "test.txt", &long_diff);

        assert!(output.contains("... 50 more lines"));
    }

    // New file display tests

    #[test]
    fn test_format_new_file_plain_has_indicator() {
        let output = format_new_file_plain("new.txt", "line1\nline2\n");

        assert!(output.contains("(new file)"));
        assert!(output.contains("new.txt"));
    }

    #[test]
    fn test_format_new_file_plain_no_diff_markers() {
        let output = format_new_file_plain("new.txt", "line1\nline2\n");

        // Should not have diff markers
        assert!(!output.contains("+line1"));
        assert!(!output.contains("-line1"));
        // Should have content without markers
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
    }

    #[test]
    fn test_format_new_file_plain_has_line_numbers() {
        let output = format_new_file_plain("new.txt", "first\nsecond\nthird\n");

        // Should have line numbers with pipe separator
        assert!(output.contains("1 │ first"));
        assert!(output.contains("2 │ second"));
        assert!(output.contains("3 │ third"));
    }

    #[test]
    fn test_format_new_file_plain_truncation() {
        let long_content: String = (0..100).map(|i| format!("line{}\n", i)).collect();

        let output = format_new_file_plain("test.txt", &long_content);

        assert!(output.contains("... 50 more lines"));
    }

    #[test]
    fn test_format_new_file_highlighted_has_indicator() {
        let processor = StreamProcessor::with_highlighting(true);

        let output =
            format_new_file_highlighted(&processor, "new.rs", "fn main() {}", Some("rust"));

        // Should have (new file) indicator
        assert!(output.contains("(new file)"));
        // Should have file path
        assert!(output.contains("new.rs"));
    }

    #[test]
    fn test_format_new_file_highlighted_has_green_background() {
        let processor = StreamProcessor::with_highlighting(true);

        let output = format_new_file_highlighted(&processor, "new.rs", "let x = 1;", Some("rust"));

        // Should contain green background ANSI code
        assert!(output.contains(GREEN_BG));
    }

    #[test]
    fn test_format_new_file_highlighted_no_diff_markers() {
        let processor = StreamProcessor::with_highlighting(true);

        let output = format_new_file_highlighted(&processor, "new.txt", "hello\nworld\n", None);

        // Should not have diff markers
        assert!(!output.contains("+hello"));
        assert!(!output.contains("-hello"));
    }

    #[test]
    fn test_format_new_file_highlighted_truncation() {
        let processor = StreamProcessor::with_highlighting(true);
        let long_content: String = (0..100)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");

        let output = format_new_file_highlighted(&processor, "test.txt", &long_content, None);

        assert!(output.contains("... 50 more lines"));
    }

    #[test]
    fn test_format_no_changes_message_new_file() {
        let processor = StreamProcessor::with_highlighting(false);

        let output = format_no_changes_message(&processor, "empty.txt", true);

        assert!(output.contains("(new file)"));
        assert!(output.contains("empty.txt"));
    }

    #[test]
    fn test_format_no_changes_message_overwrite() {
        let processor = StreamProcessor::with_highlighting(false);

        let output = format_no_changes_message(&processor, "existing.txt", false);

        assert!(output.contains("(no changes)"));
        assert!(!output.contains("(new file)"));
    }
}
