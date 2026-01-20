//! TodoWrite tool result rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};

/// Render a TodoWrite result.
pub fn render_todowrite_result(
    ctx: &RenderContext,
    is_error: bool,
    message: Option<&str>,
) -> String {
    if ctx.terminal {
        if is_error {
            format!(
                "{}✗{} {}\n",
                ansi::RED,
                ansi::RESET,
                message.unwrap_or("Failed to update todos")
            )
        } else {
            format!(
                "{}✓{} {}\n",
                ansi::GREEN,
                ansi::RESET,
                message.unwrap_or("Todos updated")
            )
        }
    } else if is_error {
        format!("! {}\n", message.unwrap_or("Failed to update todos"))
    } else {
        format!("  {}\n", message.unwrap_or("Todos updated"))
    }
}
