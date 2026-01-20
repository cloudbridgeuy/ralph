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
            ansi::DIM,
            file_path,
            ansi::RESET,
            ansi::GREEN,
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
                ansi::DIM,
                file_path,
                ansi::RESET,
                ansi::GREEN,
                ansi::RESET,
                ansi::YELLOW,
                ansi::RESET
            )
        } else {
            format!("{} (new file)\nEmpty file created\n", file_path)
        }
    } else {
        render_no_changes_message(ctx, file_path, "write")
    }
}
