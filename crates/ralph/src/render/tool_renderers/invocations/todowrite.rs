//! TodoWrite tool invocation rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::tool_renderers::types::TodoDisplayItem;

/// Render a TodoWrite tool invocation (verbose mode).
pub fn render_todowrite_invocation(ctx: &RenderContext, todos: &[TodoDisplayItem]) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!(
            "{}▶ TodoWrite{}\n",
            ctx.tools.name_fg(),
            ansi::RESET
        ));

        if todos.is_empty() {
            output.push_str(&format!(
                "  {}(clearing todo list){}\n",
                ctx.tools.label_fg(),
                ansi::RESET
            ));
        } else {
            for todo in todos {
                let status_icon = match todo.status {
                    "completed" => format!("{}✓{}", ctx.tools.ok_fg(), ansi::RESET),
                    "in_progress" => format!("{}⋯{}", ctx.tools.accent_fg(), ansi::RESET),
                    _ => format!("{}○{}", ctx.tools.label_fg(), ansi::RESET),
                };
                output.push_str(&format!("  {} {}", status_icon, todo.content));

                // Show activeForm if different from content
                if let Some(af) = todo.active_form {
                    if af != todo.content {
                        output.push_str(&format!(
                            " {}({}){}",
                            ctx.tools.label_fg(),
                            af,
                            ansi::RESET
                        ));
                    }
                }
                output.push('\n');
            }
        }
    } else {
        output.push_str("> TodoWrite\n");

        if todos.is_empty() {
            output.push_str("  (clearing todo list)\n");
        } else {
            for todo in todos {
                let status_marker = match todo.status {
                    "completed" => "[x]",
                    "in_progress" => "[~]",
                    _ => "[ ]",
                };
                output.push_str(&format!("  {} {}", status_marker, todo.content));

                // Show activeForm if different from content
                if let Some(af) = todo.active_form {
                    if af != todo.content {
                        output.push_str(&format!(" ({})", af));
                    }
                }
                output.push('\n');
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::tool_theme::test_colors;

    fn todos() -> [TodoDisplayItem<'static>; 3] {
        let item = |status| TodoDisplayItem {
            content: "t",
            status,
            active_form: None,
        };
        [item("completed"), item("in_progress"), item("pending")]
    }

    #[test]
    fn marks_use_the_context_tool_colors() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_tools(test_colors());
        let out = render_todowrite_invocation(&ctx, &todos());
        assert!(out.starts_with("\x1b[94m▶ TodoWrite"));
        assert!(out.contains("\x1b[97m✓"));
        assert!(out.contains("\x1b[91m⋯"));
        assert!(out.contains("\x1b[96m○"));
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        assert!(!render_todowrite_invocation(&ctx, &todos()).contains('\x1b'));
    }
}
