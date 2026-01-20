//! Glob tool invocation rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};

/// Render a Glob tool invocation (verbose mode).
pub fn render_glob_invocation(ctx: &RenderContext, pattern: &str, path: Option<&str>) -> String {
    // Default to current directory if path not provided
    let search_path = path.unwrap_or(".");

    if ctx.terminal {
        let mut output = String::new();
        output.push_str(&format!("{}▶ Glob{}\n", ansi::CYAN, ansi::RESET));
        output.push_str(&format!(
            "  {}Pattern:{} {}\n",
            ansi::BOLD,
            ansi::RESET,
            pattern
        ));
        output.push_str(&format!(
            "  {}Path:{} {}\n",
            ansi::DIM,
            ansi::RESET,
            search_path
        ));
        output
    } else {
        let mut output = String::new();
        output.push_str("> Glob\n");
        output.push_str(&format!("  Pattern: {}\n", pattern));
        output.push_str(&format!("  Path: {}\n", search_path));
        output
    }
}
