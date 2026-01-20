//! TodoWrite tool invocation rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::tool_renderers::types::TodoDisplayItem;

/// Render a TodoWrite tool invocation (verbose mode).
pub fn render_todowrite_invocation(ctx: &RenderContext, todos: &[TodoDisplayItem]) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!("{}▶ TodoWrite{}\n", ansi::CYAN, ansi::RESET));

        if todos.is_empty() {
            output.push_str(&format!(
                "  {}(clearing todo list){}\n",
                ansi::DIM,
                ansi::RESET
            ));
        } else {
            for todo in todos {
                let status_icon = match todo.status {
                    "completed" => format!("{}✓{}", ansi::GREEN, ansi::RESET),
                    "in_progress" => format!("{}⋯{}", ansi::YELLOW, ansi::RESET),
                    _ => format!("{}○{}", ansi::DIM, ansi::RESET),
                };
                output.push_str(&format!("  {} {}", status_icon, todo.content));

                // Show activeForm if different from content
                if let Some(af) = todo.active_form {
                    if af != todo.content {
                        output.push_str(&format!(" {}({}){}", ansi::DIM, af, ansi::RESET));
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
