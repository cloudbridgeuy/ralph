//! Grep tool invocation rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::tool_renderers::types::GrepInvocationParams;

/// Render a Grep tool invocation (verbose mode).
pub fn render_grep_invocation(ctx: &RenderContext, params: &GrepInvocationParams) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!("{}▶ Grep{}\n", ctx.tools.name_fg(), ansi::RESET));

        // Pattern with regex highlighting
        output.push_str(&format!("  {}Pattern:{} ", ansi::BOLD, ansi::RESET));
        let highlighted = ctx.highlighter.highlight(params.pattern, Some("regex"));
        output.push_str(&highlighted);
        output.push('\n');

        // Optional fields
        if let Some(p) = params.path {
            output.push_str(&format!(
                "  {}Path:{} {}\n",
                ctx.tools.label_fg(),
                ansi::RESET,
                p
            ));
        }
        if let Some(mode) = params.output_mode {
            output.push_str(&format!(
                "  {}Mode:{} {}\n",
                ctx.tools.label_fg(),
                ansi::RESET,
                mode
            ));
        }
        if let Some(g) = params.glob {
            output.push_str(&format!(
                "  {}glob:{} {}\n",
                ctx.tools.label_fg(),
                ansi::RESET,
                g
            ));
        }
        if let Some(ft) = params.file_type {
            output.push_str(&format!(
                "  {}type:{} {}\n",
                ctx.tools.label_fg(),
                ansi::RESET,
                ft
            ));
        }
        if params.case_insensitive {
            output.push_str(&format!(
                "  {}case-insensitive:{} true\n",
                ctx.tools.label_fg(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        let params = GrepInvocationParams {
            pattern: "fo+",
            path: Some("src"),
            output_mode: Some("content"),
            glob: Some("*.rs"),
            file_type: Some("rust"),
            case_insensitive: true,
        };
        assert!(!render_grep_invocation(&ctx, &params).contains('\x1b'));
    }
}
