//! Default tool result rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};

/// Render a default tool result (fallback for unknown tools).
pub fn render_default_result(ctx: &RenderContext, is_error: bool, content: Option<&str>) -> String {
    let display_content = content.unwrap_or("(no output)");

    if ctx.terminal {
        if is_error {
            format!("{}✗ Error:{} {}\n", ansi::RED, ansi::RESET, display_content)
        } else {
            format!(
                "{}✓{} {}{}{}\n",
                ansi::GREEN,
                ansi::RESET,
                ansi::DIM,
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
