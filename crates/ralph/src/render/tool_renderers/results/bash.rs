//! Bash tool result rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};

/// Render a Bash tool result.
pub fn render_bash_result(
    ctx: &RenderContext,
    is_error: bool,
    content: Option<&str>,
    truncated: bool,
) -> String {
    // Treat empty string the same as None for display purposes
    let content = content.filter(|c| !c.is_empty());

    if ctx.terminal {
        if is_error {
            let mut output = format!("{}✗ Error{}\n", ctx.tools.error_fg(), ansi::RESET);
            if let Some(c) = content {
                output.push_str(&format!("{}{}{}\n", ctx.tools.label_fg(), c, ansi::RESET));
            }
            output
        } else if let Some(c) = content {
            let mut output = format!("{}{}{}\n", ctx.tools.label_fg(), c, ansi::RESET);
            if truncated {
                output.push_str(&format!(
                    "{}(output truncated){}\n",
                    ctx.tools.label_fg(),
                    ansi::RESET
                ));
            }
            output
        } else {
            format!("{}✓ (ok){}\n", ctx.tools.ok_fg(), ansi::RESET)
        }
    } else if is_error {
        let mut output = "! Error\n".to_string();
        if let Some(c) = content {
            output.push_str(c);
            output.push('\n');
        }
        output
    } else if let Some(c) = content {
        let mut output = c.to_string();
        output.push('\n');
        if truncated {
            output.push_str("(output truncated)\n");
        }
        output
    } else {
        "(ok)\n".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::tool_theme::test_colors;

    #[test]
    fn marks_use_the_context_tool_colors() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_tools(test_colors());
        assert!(render_bash_result(&ctx, true, Some("e"), false).starts_with("\x1b[92m✗"));
        assert!(render_bash_result(&ctx, false, None, false).starts_with("\x1b[97m✓"));
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        assert!(!render_bash_result(&ctx, true, Some("e"), true).contains('\x1b'));
        assert!(!render_bash_result(&ctx, false, Some("o"), false).contains('\x1b'));
        assert!(!render_bash_result(&ctx, false, None, false).contains('\x1b'));
    }
}
