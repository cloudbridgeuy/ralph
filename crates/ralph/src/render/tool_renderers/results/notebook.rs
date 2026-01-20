//! NotebookEdit tool result rendering.

use crate::diff_highlight::highlight_with_basic_colors;
use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::tool_renderers::types::NotebookEditParams;

/// Render a NotebookEdit result.
pub fn render_notebook_edit(ctx: &RenderContext, params: &NotebookEditParams) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!(
            "{}{}{} cell {} ({}) [{}]\n",
            ansi::DIM,
            params.notebook_path,
            ansi::RESET,
            params.cell_identifier,
            params.cell_type.unwrap_or("code"),
            params.edit_mode
        ));
        output.push_str(&highlight_with_basic_colors(params.diff_content));
    } else {
        output.push_str(&format!(
            "{} cell {} ({}) [{}]\n",
            params.notebook_path,
            params.cell_identifier,
            params.cell_type.unwrap_or("code"),
            params.edit_mode
        ));
        output.push_str(params.diff_content);
    }
    output.push('\n');

    output
}
