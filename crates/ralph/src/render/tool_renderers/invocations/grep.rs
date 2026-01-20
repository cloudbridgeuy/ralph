//! Grep tool invocation rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::tool_renderers::types::GrepInvocationParams;

/// Render a Grep tool invocation (verbose mode).
pub fn render_grep_invocation(ctx: &RenderContext, params: &GrepInvocationParams) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!("{}▶ Grep{}\n", ansi::CYAN, ansi::RESET));

        // Pattern with regex highlighting
        output.push_str(&format!("  {}Pattern:{} ", ansi::BOLD, ansi::RESET));
        let highlighted = ctx.highlighter.highlight(params.pattern, Some("regex"));
        output.push_str(&highlighted);
        output.push('\n');

        // Optional fields
        if let Some(p) = params.path {
            output.push_str(&format!("  {}Path:{} {}\n", ansi::DIM, ansi::RESET, p));
        }
        if let Some(mode) = params.output_mode {
            output.push_str(&format!("  {}Mode:{} {}\n", ansi::DIM, ansi::RESET, mode));
        }
        if let Some(g) = params.glob {
            output.push_str(&format!("  {}glob:{} {}\n", ansi::DIM, ansi::RESET, g));
        }
        if let Some(ft) = params.file_type {
            output.push_str(&format!("  {}type:{} {}\n", ansi::DIM, ansi::RESET, ft));
        }
        if params.case_insensitive {
            output.push_str(&format!(
                "  {}case-insensitive:{} true\n",
                ansi::DIM,
                ansi::RESET
            ));
        }
    } else {
        output.push_str("> Grep\n");
        output.push_str(&format!("  Pattern: {}\n", params.pattern));
        if let Some(p) = params.path {
            output.push_str(&format!("  Path: {}\n", p));
        }
        if let Some(mode) = params.output_mode {
            output.push_str(&format!("  Mode: {}\n", mode));
        }
        if let Some(g) = params.glob {
            output.push_str(&format!("  glob: {}\n", g));
        }
        if let Some(ft) = params.file_type {
            output.push_str(&format!("  type: {}\n", ft));
        }
        if params.case_insensitive {
            output.push_str("  case-insensitive: true\n");
        }
    }

    output
}
