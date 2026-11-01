//! Default tool invocation rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};

/// Render a default tool invocation (fallback for unknown tools).
pub fn render_default_invocation(
    ctx: &RenderContext,
    tool_name: &str,
    key_argument: Option<&str>,
) -> String {
    if ctx.terminal {
        format!(
            "{}▶ {}{}{}\n",
            ctx.tools.name_fg(),
            tool_name,
            ansi::RESET,
            if let Some(arg) = key_argument {
                format!(" {}{}{}", ctx.tools.args_fg(), arg, ansi::RESET)
            } else {
                String::new()
            }
        )
    } else {
        format!("> {} {}\n", tool_name, key_argument.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::tool_theme::test_colors;

    #[test]
    fn name_and_args_come_from_the_context() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_tools(test_colors());
        let out = render_default_invocation(&ctx, "Foo", Some("a"));
        assert!(out.starts_with("\x1b[94m▶ Foo"));
        assert!(out.contains("\x1b[95ma"));
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        assert!(!render_default_invocation(&ctx, "Foo", Some("a")).contains('\x1b'));
    }
}
