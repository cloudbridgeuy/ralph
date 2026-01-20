//! Read tool invocation rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};

/// Render a Read tool invocation (verbose mode).
pub fn render_read_invocation(
    ctx: &RenderContext,
    file_path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!("{}▶ Read{}\n", ansi::CYAN, ansi::RESET));
        output.push_str(&format!("  {}{}{}\n", ansi::DIM, file_path, ansi::RESET));

        if let Some(off) = offset {
            output.push_str(&format!("  offset: {}{}{}\n", ansi::DIM, off, ansi::RESET));
        }
        if let Some(lim) = limit {
            output.push_str(&format!("  limit: {}{}{}\n", ansi::DIM, lim, ansi::RESET));
        }
    } else {
        output.push_str("> Read\n");
        output.push_str(&format!("  {}\n", file_path));
        if let Some(off) = offset {
            output.push_str(&format!("  offset: {}\n", off));
        }
        if let Some(lim) = limit {
            output.push_str(&format!("  limit: {}\n", lim));
        }
    }

    output
}
