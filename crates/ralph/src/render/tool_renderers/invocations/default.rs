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
            ansi::CYAN,
            tool_name,
            ansi::RESET,
            if let Some(arg) = key_argument {
                format!(" {}{}{}", ansi::DIM, arg, ansi::RESET)
            } else {
                String::new()
            }
        )
    } else {
        format!("> {} {}\n", tool_name, key_argument.unwrap_or_default())
    }
}
