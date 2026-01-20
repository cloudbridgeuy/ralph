//! Bash tool invocation rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};

/// Render a Bash tool invocation.
///
/// Single-line commands show inline; multi-line commands are wrapped in code blocks.
pub fn render_bash_invocation(ctx: &RenderContext, command: &str) -> String {
    let is_multiline = command.contains('\n');

    if ctx.terminal {
        let mut output = String::new();
        output.push_str(&format!("{}▶ Bash{}\n", ansi::CYAN, ansi::RESET));

        if is_multiline {
            output.push_str("```sh\n");
            let highlighted = ctx.highlighter.highlight(command, Some("sh"));
            output.push_str(&highlighted);
            if !highlighted.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("```\n");
        } else {
            output.push_str("  ");
            let highlighted = ctx.highlighter.highlight(command, Some("sh"));
            let trimmed = highlighted.trim_end_matches(ansi::RESET);
            output.push_str(trimmed);
            output.push_str(&format!("{}\n", ansi::RESET));
        }

        output
    } else if is_multiline {
        let mut output = String::new();
        output.push_str("> Bash\n```sh\n");
        output.push_str(command);
        if !command.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("```\n");
        output
    } else {
        format!("> Bash\n  {}\n", command)
    }
}
