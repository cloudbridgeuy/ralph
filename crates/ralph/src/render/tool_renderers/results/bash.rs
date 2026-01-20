//! Bash tool result rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};

/// Render a Bash tool result.
pub fn render_bash_result(
    ctx: &RenderContext,
    is_error: bool,
    content: Option<&str>,
    truncated: bool,
) -> String {
    // Treat empty string the same as None for display purposes
    let content = content.filter(|c| !c.is_empty());

    if ctx.terminal {
        if is_error {
            let mut output = format!("{}✗ Error{}\n", ansi::RED, ansi::RESET);
            if let Some(c) = content {
                output.push_str(&format!("{}{}{}\n", ansi::DIM, c, ansi::RESET));
            }
            output
        } else if let Some(c) = content {
            let mut output = format!("{}{}{}\n", ansi::DIM, c, ansi::RESET);
            if truncated {
                output.push_str(&format!("{}(output truncated){}\n", ansi::DIM, ansi::RESET));
            }
            output
        } else {
            format!("{}✓ (ok){}\n", ansi::GREEN, ansi::RESET)
        }
    } else if is_error {
        let mut output = "! Error\n".to_string();
        if let Some(c) = content {
            output.push_str(c);
            output.push('\n');
        }
        output
    } else if let Some(c) = content {
        let mut output = c.to_string();
        output.push('\n');
        if truncated {
            output.push_str("(output truncated)\n");
        }
        output
    } else {
        "(ok)\n".to_string()
    }
}
