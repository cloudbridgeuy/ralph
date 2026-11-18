mod edit;
mod notebookedit;
mod write;

pub use edit::generate_diff_from_snapshot as edit_diff_from_snapshot;
pub use edit::{format_before_after, format_diff_output, format_no_changes_message};
pub use notebookedit::format_notebook_result;
pub use notebookedit::generate_diff_from_snapshot as notebook_diff_from_snapshot;
pub use write::{
    format_before_after_display as format_write_before_after, format_new_file_display,
    format_no_changes_message as format_write_no_changes,
};

use super::utils::truncate_string;
use crate::ansi::RESET;
use crate::render::RenderContext;

pub fn format_tool_error(ctx: &RenderContext, tool_name: &str, content: Option<&str>) -> String {
    let content = content.map_or_else(
        || format!("({} failed)", tool_name.to_lowercase()),
        |c| truncate_string(c, 200),
    );
    if ctx.terminal {
        format!(
            "{}✗ {tool_name} error:{RESET} {content}\n",
            ctx.tools.error_fg()
        )
    } else {
        format!("! {tool_name} error: {content}\n")
    }
}
