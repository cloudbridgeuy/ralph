//! Write tool result rendering.

use crate::render::tool_renderers::context::{ansi, RenderContext};
use crate::render::tool_renderers::results::edit::{
    render_content_block, render_no_changes_message,
};
use crate::render::utils::extract_language_from_path;

/// Render a Write new file result.
pub fn render_write_new_file(ctx: &RenderContext, file_path: &str, content: &str) -> String {
    let mut output = String::new();

    // File header with (new file) indicator
    if ctx.terminal {
        output.push_str(&format!(
            "{}{}{} {}(new file){}\n",
            ctx.tools.label_fg(),
            file_path,
            ansi::RESET,
            ctx.tools.ok_fg(),
            ansi::RESET
        ));
    } else {
        output.push_str(&format!("{} (new file)\n", file_path));
    }

    // Content with green background line numbers
    let language = extract_language_from_path(file_path);
    output.push_str(&render_content_block(ctx, content, language, false));

    output
}

/// Render a Write no changes result.
pub fn render_write_no_changes(ctx: &RenderContext, file_path: &str, is_new_file: bool) -> String {
    if is_new_file {
        if ctx.terminal {
            format!(
                "{}{}{} {}(new file){}\n{}⚠ Empty file created{}\n",
                ctx.tools.label_fg(),
                file_path,
                ansi::RESET,
                ctx.tools.ok_fg(),
                ansi::RESET,
                ctx.tools.accent_fg(),
                ansi::RESET
            )
        } else {
            format!("{} (new file)\nEmpty file created\n", file_path)
        }
    } else {
        render_no_changes_message(ctx, file_path, "write")
    }
}

#[cfg(test)]
mod tool_color_tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::tool_theme::test_colors;

    #[test]
    fn header_uses_the_context_tool_colors() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::terminal(&highlighter).with_tools(test_colors());
        let out = render_write_new_file(&ctx, "a.rs", "x");
        assert!(out.starts_with("\x1b[96ma.rs"));
        assert!(out.contains("\x1b[97m(new file)"));
        assert!(!out.contains("\x1b[90m"));
    }

    #[test]
    fn plain_context_has_no_escape_byte() {
        let highlighter = Highlighter::new();
        let ctx = RenderContext::plain(&highlighter);
        assert!(!render_write_new_file(&ctx, "a.rs", "x").contains('\x1b'));
        assert!(!render_write_no_changes(&ctx, "a.rs", true).contains('\x1b'));
        assert!(!render_write_no_changes(&ctx, "a.rs", false).contains('\x1b'));
    }
}
