//! Write tool result formatting.
//!
//! Formats Write tool results with generated diffs and syntax highlighting.
//!
//! Claude CLI's Write tool creates or overwrites files, returning success messages.
//! To show meaningful diffs, we:
//! 1. Capture file content before the Write tool executes (via WriteSnapshot)
//! 2. Read the file again after the write completes
//! 3. For new files: display content with syntax highlighting and green line numbers
//! 4. For overwrites: display before/after blocks with colored line numbers
//! 5. Display with syntax highlighting

use std::fs;

use super::super::processor::StreamProcessor;
use super::super::types::WriteSnapshot;
use super::super::utils::extract_language_from_path;

/// Maximum lines to show in a new file display before truncating.
const MAX_NEW_FILE_LINES: usize = 50;

/// Maximum lines to show in a before/after block before truncating.
const MAX_BLOCK_LINES: usize = 30;

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
/// Yellow foreground (for indicators)
const YELLOW: &str = "\x1b[33m";
/// Green foreground (for checkmark)
const GREEN: &str = "\x1b[32m";

/// Format a Write tool result using a snapshot.
///
/// This is called when we have captured a snapshot before the write.
/// For new files, displays clean syntax-highlighted content with green line numbers.
/// For overwrites, displays before/after blocks with colored line numbers.
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

    // Handle overwrites: show before/after blocks
    let before_content = snapshot.content.as_deref().unwrap_or("");
    if let Ok(after_content) = fs::read_to_string(&snapshot.file_path) {
        // Skip if no changes
        if before_content == after_content {
            return format_no_changes_message(processor, &snapshot.file_path, false);
        }
        format_before_after_display(
            processor,
            &snapshot.file_path,
            before_content,
            &after_content,
        )
    } else {
        // Couldn't read file - show simple success message
        format_no_changes_message(processor, &snapshot.file_path, false)
    }
}

/// Format before/after display for file overwrites.
fn format_before_after_display(
    processor: &StreamProcessor,
    file_path: &str,
    before: &str,
    after: &str,
) -> String {
    let language = extract_language_from_path(file_path);

    if processor.highlighting_enabled {
        format_before_after_highlighted(processor, file_path, before, after, language)
    } else {
        format_before_after_plain(file_path, before, after)
    }
}

/// Format before/after blocks with syntax highlighting and colored backgrounds.
fn format_before_after_highlighted(
    processor: &StreamProcessor,
    file_path: &str,
    before: &str,
    after: &str,
    language: Option<&'static str>,
) -> String {
    let mut output = String::new();

    // File path header with box drawing
    output.push_str(&format!("{}── {} ──{}\n", CYAN, file_path, RESET));

    // Before block
    output.push_str(&format_highlighted_block(
        processor, "Before:", before, language, RED_BG,
    ));

    // Horizontal separator
    output.push_str(&format!("{}─────────────────────────{}\n", DIM, RESET));

    // After block
    output.push_str(&format_highlighted_block(
        processor, "After:", after, language, GREEN_BG,
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
fn format_before_after_plain(file_path: &str, before: &str, after: &str) -> String {
    let mut output = String::new();

    // File path header
    output.push_str(&format!("-- {} --\n", file_path));

    // Before block
    output.push_str(&format_plain_block("Before:", before));

    // Horizontal separator
    output.push_str("-------------------------\n");

    // After block
    output.push_str(&format_plain_block("After:", after));

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

#[cfg(test)]
mod tests {
    use super::*;

    // Before/after display tests

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
    fn test_format_before_after_plain_empty_before() {
        let output = format_before_after_plain("test.rs", "", "new code");

        assert!(output.contains("(empty)"));
        assert!(output.contains("new code"));
    }

    #[test]
    fn test_format_before_after_plain_empty_after() {
        let output = format_before_after_plain("test.rs", "old code", "");

        assert!(output.contains("old code"));
        assert!(output.contains("(empty)"));
    }

    #[test]
    fn test_format_before_after_plain_has_separator() {
        let output = format_before_after_plain("test.rs", "old", "new");

        // Should have horizontal separator
        assert!(output.contains("-------------------------"));
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
        // Should contain both background color codes
        assert!(output.contains(RED_BG));
        assert!(output.contains(GREEN_BG));
    }

    #[test]
    fn test_format_before_after_highlighted_has_separator() {
        let processor = StreamProcessor::with_highlighting(true);

        let output =
            format_before_after_highlighted(&processor, "test.rs", "old", "new", Some("rust"));

        // Should have horizontal separator (dim)
        assert!(output.contains("─────────────────────────"));
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
    fn test_format_plain_block_has_line_numbers() {
        let output = format_plain_block("Test:", "first\nsecond\nthird");

        assert!(output.contains("1 │ first"));
        assert!(output.contains("2 │ second"));
        assert!(output.contains("3 │ third"));
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
