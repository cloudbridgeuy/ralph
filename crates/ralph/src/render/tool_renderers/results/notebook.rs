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
            ctx.tools.label_fg(),
            params.notebook_path,
            ansi::RESET,
            params.cell_identifier,
            params.cell_type.unwrap_or("code"),
            params.edit_mode
        ));
        output.push_str(&highlight_with_basic_colors(params.diff_content, &ctx.diff));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_theme::test_colors;
    use crate::highlight::Highlighter;

    fn params(diff_content: &str) -> NotebookEditParams<'_> {
        NotebookEditParams {
            notebook_path: "n.ipynb",
            cell_identifier: "0",
            cell_type: None,
            edit_mode: "replace",
            diff_content,
        }
    }

    #[test]
    fn diff_uses_the_context_diff_colors() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_diff(test_colors());
        assert!(render_notebook_edit(&ctx, &params("+x")).contains("\x1b[94m+x"));
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        assert!(!render_notebook_edit(&ctx, &params("+x\n-y")).contains('\x1b'));
    }
}
