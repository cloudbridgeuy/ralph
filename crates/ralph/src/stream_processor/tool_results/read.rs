//! Read tool result formatting (verbose mode).
//!
//! Formats Read tool results with syntax highlighting.

use ralph_core::stream::{ToolInvocation, ToolResult};

use super::super::processor::StreamProcessor;
use super::super::utils::{extract_language_from_path, truncate_string};

/// Normalize Claude CLI's `cat -n` line number format to a cleaner pipe-separated format.
///
/// Transforms lines from:
/// - `     1\tcontent` → `1 │ content`
/// - `    12\tcontent` → `12 │ content`
/// - `   123\tcontent` → `123 │ content`
///
/// Lines that don't match the `cat -n` pattern are passed through unchanged.
fn normalize_cat_n_format(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            // Pattern: leading whitespace + digits + tab + content
            // Claude CLI's cat -n output uses tab between number and content
            if let Some(tab_pos) = line.find('\t') {
                let prefix = &line[..tab_pos];
                let rest = &line[tab_pos + 1..];

                // Check if prefix is whitespace followed by digits
                let trimmed = prefix.trim_start();
                if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
                    return format!("{} │ {}", trimmed, rest);
                }
            }
            // Pass through unchanged if pattern doesn't match
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Format a Read tool result with verbose output.
///
/// In verbose mode, the file content is displayed with syntax highlighting
/// based on the file extension. Line numbers from Claude CLI's `cat -n` format
/// are normalized to use pipe separators (e.g., `1 │ content` instead of `     1→content`).
pub fn format_read_tool_result_verbose(
    processor: &StreamProcessor,
    invocation: ToolInvocation,
    result: &ToolResult,
) -> String {
    const MAX_CONTENT_LINES: usize = 100;

    if result.is_error {
        // Error case - show error message
        let error_content = result
            .content
            .as_ref()
            .map(|c| truncate_string(c, 200))
            .unwrap_or_else(|| "(read failed)".to_string());

        return if processor.highlighting_enabled {
            format!("\x1b[31m✗ Read error:\x1b[0m {}\n", error_content)
        } else {
            format!("! Read error: {}\n", error_content)
        };
    }

    let raw_content = result.content.as_deref().unwrap_or("");

    // Empty result
    if raw_content.is_empty() {
        return if processor.highlighting_enabled {
            "\x1b[90m(empty file)\x1b[0m\n".to_string()
        } else {
            "(empty file)\n".to_string()
        };
    }

    // Check for binary file indicator
    if raw_content.contains("(binary file)") || raw_content.starts_with('\u{0}') {
        return if processor.highlighting_enabled {
            "\x1b[90m(binary file)\x1b[0m\n".to_string()
        } else {
            "(binary file)\n".to_string()
        };
    }

    // Extract file path for language detection
    let file_path = invocation
        .input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Extract language from file extension
    let language = extract_language_from_path(file_path);

    // Normalize cat -n format before processing
    // This transforms "     1\tcontent" to "1│ content"
    let content = normalize_cat_n_format(raw_content);

    // Count lines for potential truncation
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let (display_lines, truncated) = if line_count > MAX_CONTENT_LINES {
        (&lines[..MAX_CONTENT_LINES], true)
    } else {
        (&lines[..], false)
    };

    if processor.highlighting_enabled {
        let mut output = String::new();

        // Results header showing line count
        let line_word = if line_count == 1 { "line" } else { "lines" };
        output.push_str(&format!(
            "\x1b[32m✓\x1b[0m \x1b[90m{} {}\x1b[0m\n",
            line_count, line_word
        ));

        // Apply syntax highlighting to the content
        let content_to_highlight = display_lines.join("\n");
        let highlighted = if language.is_some() {
            processor
                .code_highlighter
                .highlight(&content_to_highlight, language)
        } else {
            content_to_highlight.clone()
        };

        // Display highlighted content with indentation
        for line in highlighted.lines() {
            output.push_str(&format!("  {}\n", line));
        }

        if truncated {
            output.push_str(&format!(
                "\x1b[90m... {} more lines\x1b[0m\n",
                line_count - MAX_CONTENT_LINES
            ));
        }

        output
    } else {
        // Plain text format
        let mut output = String::new();

        let line_word = if line_count == 1 { "line" } else { "lines" };
        output.push_str(&format!("{} {}\n", line_count, line_word));

        for line in display_lines {
            output.push_str(&format!("  {}\n", line));
        }

        if truncated {
            output.push_str(&format!(
                "... {} more lines\n",
                line_count - MAX_CONTENT_LINES
            ));
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_cat_n_format_single_digit() {
        let input = "     1\tfn main() {";
        let expected = "1 │ fn main() {";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_double_digit() {
        let input = "    12\t    println!(\"hello\");";
        let expected = "12 │     println!(\"hello\");";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_triple_digit() {
        let input = "   123\t}";
        let expected = "123 │ }";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_multiple_lines() {
        let input = "     1\tfn main() {\n     2\t    println!(\"hello\");\n     3\t}";
        let expected = "1 │ fn main() {\n2 │     println!(\"hello\");\n3 │ }";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_pass_through_no_tab() {
        // Lines without tabs should pass through unchanged
        let input = "This is just plain text";
        assert_eq!(normalize_cat_n_format(input), input);
    }

    #[test]
    fn test_normalize_cat_n_format_pass_through_non_numeric_prefix() {
        // Lines with tabs but non-numeric prefix should pass through
        let input = "abc\tcontent";
        assert_eq!(normalize_cat_n_format(input), input);
    }

    #[test]
    fn test_normalize_cat_n_format_preserves_content_with_tabs() {
        // Content after the first tab should be preserved, including tabs
        let input = "     1\tfield1\tfield2";
        let expected = "1 │ field1\tfield2";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_empty_content() {
        // Line with just a number and tab (empty content)
        let input = "     1\t";
        let expected = "1 │ ";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_empty_string() {
        let input = "";
        assert_eq!(normalize_cat_n_format(input), "");
    }

    #[test]
    fn test_normalize_cat_n_format_mixed_lines() {
        // Mix of cat-n formatted lines and regular lines
        let input = "     1\tcode line\nregular text\n     2\tmore code";
        let expected = "1 │ code line\nregular text\n2 │ more code";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_with_offset() {
        // Simulates partial file read with offset - line numbers don't start at 1
        let input = "    50\tline fifty\n    51\tline fifty-one";
        let expected = "50 │ line fifty\n51 │ line fifty-one";
        assert_eq!(normalize_cat_n_format(input), expected);
    }
}
