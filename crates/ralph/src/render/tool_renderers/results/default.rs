//! Default tool result rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};

/// Render a default tool result (fallback for unknown tools).
pub fn render_default_result(ctx: &RenderContext, is_error: bool, content: Option<&str>) -> String {
    let display_content = content.unwrap_or("(no output)");

    if ctx.terminal {
        if is_error {
            format!(
                "{}✗ Error:{} {}\n",
                ctx.tools.error_fg(),
                ansi::RESET,
                display_content
            )
        } else {
            format!(
                "{}✓{} {}{}{}\n",
                ctx.tools.ok_fg(),
                ansi::RESET,
                ctx.tools.label_fg(),
                display_content,
                ansi::RESET
            )
        }
    } else if is_error {
        format!("! Error: {}\n", display_content)
    } else {
        format!("  {}\n", display_content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::tool_theme::test_colors;

    #[test]
    fn colors_come_from_the_context() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_tools(test_colors());
        assert!(render_default_result(&ctx, false, Some("x")).starts_with("\x1b[97m✓"));
        assert!(render_default_result(&ctx, true, Some("x")).starts_with("\x1b[92m✗"));
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        assert!(!render_default_result(&ctx, false, Some("x")).contains('\x1b'));
        assert!(!render_default_result(&ctx, true, Some("x")).contains('\x1b'));
    }
}
