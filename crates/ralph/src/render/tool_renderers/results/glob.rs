//! Glob tool result rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::utils::group_files_by_directory;

/// Render a Glob tool result (verbose mode).
pub fn render_glob_result(
    ctx: &RenderContext,
    file_count: usize,
    content: &str,
    truncated: bool,
) -> String {
    const MAX_RESULT_LINES: usize = 200;

    // Empty result
    if content.is_empty() {
        return if ctx.terminal {
            format!("{}(no matches){}\n", ansi::DIM, ansi::RESET)
        } else {
            "(no matches)\n".to_string()
        };
    }

    // Parse file paths from content
    let files: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();

    // Group files by directory for readability
    let grouped = group_files_by_directory(&files);

    // Determine if we need to truncate based on total display lines
    let total_display_lines: usize = grouped
        .values()
        .map(|paths| paths.len() + 1) // +1 for directory header
        .sum();
    let should_truncate = truncated || total_display_lines > MAX_RESULT_LINES;

    let mut output = String::new();

    if ctx.terminal {
        // Results header showing match count
        let file_word = if file_count == 1 { "file" } else { "files" };
        output.push_str(&format!(
            "{}✓{} {}{} {} matched{}\n",
            ansi::GREEN,
            ansi::RESET,
            ansi::DIM,
            file_count,
            file_word,
            ansi::RESET
        ));

        // Display files grouped by directory
        let mut lines_shown = 0;
        for (dir, paths) in &grouped {
            if should_truncate && lines_shown >= MAX_RESULT_LINES {
                break;
            }

            // Directory header
            if dir.is_empty() {
                output.push_str(&format!("  {}.{}\n", ansi::BOLD, ansi::RESET));
            } else {
                output.push_str(&format!("  {}{}/{}\n", ansi::BOLD, dir, ansi::RESET));
            }
            lines_shown += 1;

            // Files in this directory
            for path in paths {
                if should_truncate && lines_shown >= MAX_RESULT_LINES {
                    break;
                }
                let filename = path.rsplit('/').next().unwrap_or(path);
                output.push_str(&format!("    {}{}{}\n", ansi::DIM, filename, ansi::RESET));
                lines_shown += 1;
            }
        }

        if should_truncate && lines_shown < file_count {
            output.push_str(&format!(
                "{}... {} more files{}\n",
                ansi::DIM,
                file_count.saturating_sub(lines_shown),
                ansi::RESET
            ));
        }
    } else {
        // Plain text format
        let file_word = if file_count == 1 { "file" } else { "files" };
        output.push_str(&format!("{} {} matched\n", file_count, file_word));

        let mut lines_shown = 0;
        for (dir, paths) in &grouped {
            if should_truncate && lines_shown >= MAX_RESULT_LINES {
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
                if should_truncate && lines_shown >= MAX_RESULT_LINES {
                    break;
                }
                let filename = path.rsplit('/').next().unwrap_or(path);
                output.push_str(&format!("    {}\n", filename));
                lines_shown += 1;
            }
        }

        if should_truncate && lines_shown < file_count {
            output.push_str(&format!(
                "... {} more files\n",
                file_count.saturating_sub(lines_shown)
            ));
        }
    }

    output
}
