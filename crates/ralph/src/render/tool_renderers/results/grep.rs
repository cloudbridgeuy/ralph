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
            format!("{}(no matches){}\n", ctx.tools.label_fg(), ansi::RESET)
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
            ctx.tools.ok_fg(),
            ansi::RESET,
            ctx.tools.label_fg(),
            match_count,
            match_word,
            ansi::RESET
        ));

        // Format based on output mode
        match output_mode {
            "files_with_matches" => {
                for line in display_lines {
                    output.push_str(&format!(
                        "  {}{}{}\n",
                        ctx.tools.label_fg(),
                        line,
                        ansi::RESET
                    ));
                }
            }
            "content" => {
                for line in display_lines {
                    let highlighted_line = highlight_grep_match(line, &ctx.tools);
                    output.push_str(&format!("  {}\n", highlighted_line));
                }
            }
            _ => {
                // count mode and other modes
                for line in display_lines {
                    output.push_str(&format!(
                        "  {}{}{}\n",
                        ctx.tools.label_fg(),
                        line,
                        ansi::RESET
                    ));
                }
            }
        }

        if truncated {
            output.push_str(&format!(
                "{}... {} more lines{}\n",
                ctx.tools.label_fg(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::tool_theme::test_colors;

    #[test]
    fn match_colors_come_from_the_context() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_tools(test_colors());
        let out = render_grep_result(&ctx, 1, "content", "a.rs:1:hit");
        assert!(out.contains("\x1b[97m✓"));
        assert!(out.contains("\x1b[30ma.rs:1:\x1b[0m\x1b[37mhit"));
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        for mode in ["content", "files_with_matches", "count"] {
            assert!(!render_grep_result(&ctx, 1, mode, "a.rs:1:hit").contains('\x1b'));
        }
        assert!(!render_grep_result(&ctx, 0, "content", "").contains('\x1b'));
    }
}
