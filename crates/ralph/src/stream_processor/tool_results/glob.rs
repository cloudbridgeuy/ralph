//! Glob tool result formatting (verbose mode).
//!
//! Formats Glob tool results with all matched files displayed.

use ralph_core::stream::ToolResult;

use super::super::processor::StreamProcessor;
use super::super::utils::truncate_string;
use crate::render::group_files_by_directory;

/// Format a Glob tool result with verbose output.
///
/// In verbose mode, all matched files are displayed without truncation,
/// grouped by directory for readability, with a total match count.
pub fn format_glob_tool_result_verbose(processor: &StreamProcessor, result: &ToolResult) -> String {
    const MAX_RESULT_LINES: usize = 200;

    if result.is_error {
        // Error case - show error message
        let error_content = result
            .content
            .as_ref()
            .map(|c| truncate_string(c, 200))
            .unwrap_or_else(|| "(glob failed)".to_string());

        return if processor.highlighting_enabled {
            format!("\x1b[31m✗ Glob error:\x1b[0m {}\n", error_content)
        } else {
            format!("! Glob error: {}\n", error_content)
        };
    }

    let content = result.content.as_deref().unwrap_or("");

    // Empty result
    if content.is_empty() {
        return if processor.highlighting_enabled {
            "\x1b[90m(no matches)\x1b[0m\n".to_string()
        } else {
            "(no matches)\n".to_string()
        };
    }

    // Parse file paths from content
    let files: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    let file_count = files.len();

    // Group files by directory for readability
    let grouped = group_files_by_directory(&files);

    // Determine if we need to truncate
    let total_display_lines: usize = grouped
        .values()
        .map(|paths| paths.len() + 1) // +1 for directory header
        .sum();
    let truncated = total_display_lines > MAX_RESULT_LINES;

    if processor.highlighting_enabled {
        let mut output = String::new();

        // Results header showing match count
        let file_word = if file_count == 1 { "file" } else { "files" };
        output.push_str(&format!(
            "\x1b[32m✓\x1b[0m \x1b[90m{} {} matched\x1b[0m\n",
            file_count, file_word
        ));

        // Display files grouped by directory
        let mut lines_shown = 0;
        for (dir, paths) in &grouped {
            if truncated && lines_shown >= MAX_RESULT_LINES {
                break;
            }

            // Directory header
            if dir.is_empty() {
                output.push_str("  \x1b[1m.\x1b[0m\n");
            } else {
                output.push_str(&format!("  \x1b[1m{}/\x1b[0m\n", dir));
            }
            lines_shown += 1;

            // Files in this directory
            for path in paths {
                if truncated && lines_shown >= MAX_RESULT_LINES {
                    break;
                }
                // Extract just the filename part
                let filename = path.rsplit('/').next().unwrap_or(path);
                output.push_str(&format!("    \x1b[90m{}\x1b[0m\n", filename));
                lines_shown += 1;
            }
        }

        if truncated {
            output.push_str(&format!(
                "\x1b[90m... {} more files\x1b[0m\n",
                file_count.saturating_sub(lines_shown)
            ));
        }

        output
    } else {
        // Plain text format
        let mut output = String::new();

        let file_word = if file_count == 1 { "file" } else { "files" };
        output.push_str(&format!("{} {} matched\n", file_count, file_word));

        let mut lines_shown = 0;
        for (dir, paths) in &grouped {
            if truncated && lines_shown >= MAX_RESULT_LINES {
                break;
            }

            // Directory header
            if dir.is_empty() {
                output.push_str("  .\n");
            } else {
                output.push_str(&format!("  {}/\n", dir));
            }
            lines_shown += 1;

            // Files in this directory
            for path in paths {
                if truncated && lines_shown >= MAX_RESULT_LINES {
                    break;
                }
                let filename = path.rsplit('/').next().unwrap_or(path);
                output.push_str(&format!("    {}\n", filename));
                lines_shown += 1;
            }
        }

        if truncated {
            output.push_str(&format!(
                "... {} more files\n",
                file_count.saturating_sub(lines_shown)
            ));
        }

        output
    }
}

// Tests for group_files_by_directory are in crate::render::utils
