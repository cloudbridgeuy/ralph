//! Grep tool result rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::utils::highlight_grep_match;

/// Render a Grep tool result (verbose mode).
pub fn render_grep_result(
    ctx: &RenderContext,
    match_count: usize,
    output_mode: &str,
    content: &str,
) -> String {
    const MAX_RESULT_LINES: usize = 100;

    // Empty result
    if content.is_empty() {
        return if ctx.terminal {
            format!("{}(no matches){}\n", ansi::DIM, ansi::RESET)
        } else {
            "(no matches)\n".to_string()
        };
    }

    // Count lines for potential truncation
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let (display_lines, truncated) = if line_count > MAX_RESULT_LINES {
        (&lines[..MAX_RESULT_LINES], true)
    } else {
        (&lines[..], false)
    };

    let mut output = String::new();

    if ctx.terminal {
        // Results header showing match count
        let match_word = if match_count == 1 { "match" } else { "matches" };
        output.push_str(&format!(
            "{}✓{} {}{} {}{}\n",
            ansi::GREEN,
            ansi::RESET,
            ansi::DIM,
            match_count,
            match_word,
            ansi::RESET
        ));

        // Format based on output mode
        match output_mode {
            "files_with_matches" => {
                for line in display_lines {
                    output.push_str(&format!("  {}{}{}\n", ansi::DIM, line, ansi::RESET));
                }
            }
            "content" => {
                for line in display_lines {
                    let highlighted_line = highlight_grep_match(line);
                    output.push_str(&format!("  {}\n", highlighted_line));
                }
            }
            _ => {
                // count mode and other modes
                for line in display_lines {
                    output.push_str(&format!("  {}{}{}\n", ansi::DIM, line, ansi::RESET));
                }
            }
        }

        if truncated {
            output.push_str(&format!(
                "{}... {} more lines{}\n",
                ansi::DIM,
                line_count - MAX_RESULT_LINES,
                ansi::RESET
            ));
        }
    } else {
        // Plain text format
        let match_word = if match_count == 1 { "match" } else { "matches" };
        output.push_str(&format!("{} {}\n", match_count, match_word));

        for line in display_lines {
            output.push_str(&format!("  {}\n", line));
        }

        if truncated {
            output.push_str(&format!(
                "... {} more lines\n",
                line_count - MAX_RESULT_LINES
            ));
        }
    }

    output
}
