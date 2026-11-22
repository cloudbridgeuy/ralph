use ralph_core::chunk::ChunkType;

use super::tool_results::{
    format_before_after, format_diff_output, format_new_file_display, format_no_changes_message,
    format_notebook_result, format_tool_error, format_write_before_after, format_write_no_changes,
};
use crate::markdown::MarkdownTheme;
use crate::render::image::lookup_in;
use crate::render::tool_renderers::render_script_invocation;
use crate::render::{
    render_bash_invocation, render_bash_result, render_default_invocation, render_default_result,
    render_glob_invocation, render_glob_result, render_grep_invocation, render_grep_result,
    render_read_invocation, render_read_result, render_text_block_with,
    render_todowrite_invocation, render_todowrite_result, GrepInvocationParams, NotebookEditParams,
    RenderContext, TodoDisplayItem,
};
use ralph_core::stream::blocks::{truncate_multiline, truncate_string};
use ralph_core::stream::blocks::{
    OutputBlock, ToolInvocationBlock, ToolInvocationVariant, ToolResultBlock, ToolResultVariant,
};

const MAX_BASH_LINES: usize = 30;
const MAX_INLINE_CHARS: usize = 200;
const MAX_ARGUMENT_CHARS: usize = 60;

pub fn render_block(
    ctx: &RenderContext,
    block: &OutputBlock,
    markdown: Option<&MarkdownTheme>,
) -> String {
    match block {
        OutputBlock::Text(text) => {
            let mut output =
                render_text_block_with(ctx, &text.chunk, markdown, &lookup_in(&text.images));
            if !matches!(text.chunk.chunk_type, ChunkType::Prose) {
                output.push('\n');
            }
            output
        }
        OutputBlock::Separator => "\n".to_string(),
        OutputBlock::ToolInvocation(invocation) => render_invocation(ctx, invocation),
        OutputBlock::ToolResult(result) => render_result(ctx, result),
    }
}

fn render_invocation(ctx: &RenderContext, block: &ToolInvocationBlock) -> String {
    match &block.variant {
        ToolInvocationVariant::Bash { command, .. } => render_bash_invocation(ctx, command),
        ToolInvocationVariant::Grep {
            pattern,
            path,
            output_mode,
            glob,
            file_type,
            case_insensitive,
        } => render_grep_invocation(
            ctx,
            &GrepInvocationParams {
                pattern,
                path: path.as_deref(),
                output_mode: output_mode.as_deref(),
                glob: glob.as_deref(),
                file_type: file_type.as_deref(),
                case_insensitive: *case_insensitive,
            },
        ),
        ToolInvocationVariant::Read {
            file_path,
            offset,
            limit,
        } => render_read_invocation(ctx, file_path, *offset, *limit),
        ToolInvocationVariant::Glob { pattern, path } => {
            render_glob_invocation(ctx, pattern, path.as_deref())
        }
        ToolInvocationVariant::TodoWrite { todos } => {
            let items: Vec<TodoDisplayItem> = todos
                .iter()
                .map(|todo| TodoDisplayItem {
                    content: &todo.content,
                    status: &todo.status,
                    active_form: todo.active_form.as_deref(),
                })
                .collect();
            render_todowrite_invocation(ctx, &items)
        }
        ToolInvocationVariant::Script {
            code,
            allow_writes,
            allow_spend,
            timeout_secs,
        } => render_script_invocation(ctx, code, *allow_writes, *allow_spend, *timeout_secs),
        ToolInvocationVariant::Default {
            key_argument,
            is_path,
        } => {
            let argument = key_argument.as_ref().map(|arg| match is_path {
                true => arg.clone(),
                false => truncate_string(arg, MAX_ARGUMENT_CHARS),
            });
            render_default_invocation(ctx, &block.tool_name, argument.as_deref())
        }
    }
}

fn render_result(ctx: &RenderContext, block: &ToolResultBlock) -> String {
    let is_error = block.is_error;
    match &block.variant {
        ToolResultVariant::Bash { content, .. } => {
            let shown = content
                .as_deref()
                .map(|c| truncate_multiline(c, MAX_BASH_LINES));
            render_bash_result(
                ctx,
                is_error,
                shown.as_ref().map(|(c, _)| c.as_str()),
                shown.as_ref().is_some_and(|(_, cut)| *cut),
            )
        }
        ToolResultVariant::EditBeforeAfter {
            file_path,
            old_content,
            new_content,
        } => format_before_after(ctx, file_path, old_content, new_content),
        ToolResultVariant::EditDiff {
            file_path,
            diff_content,
        } => format_diff_output(ctx, file_path, diff_content),
        ToolResultVariant::EditNoChanges { file_path } => format_no_changes_message(ctx, file_path),
        ToolResultVariant::WriteNewFile { file_path, content } => {
            format_new_file_display(ctx, file_path, content)
        }
        ToolResultVariant::WriteOverwrite {
            file_path,
            before_content,
            after_content,
        } => format_write_before_after(ctx, file_path, before_content, after_content),
        ToolResultVariant::WriteNoChanges {
            file_path,
            is_new_file,
        } => format_write_no_changes(ctx, file_path, *is_new_file),
        ToolResultVariant::Read {
            file_path,
            content,
            line_count,
            truncated,
        } => render_read_result(ctx, file_path, content, *line_count, *truncated),
        ToolResultVariant::Grep {
            match_count,
            output_mode,
            content,
        } => render_grep_result(ctx, *match_count, output_mode, content),
        ToolResultVariant::Glob {
            file_count,
            content,
            truncated,
        } => render_glob_result(ctx, *file_count, content, *truncated),
        ToolResultVariant::TodoWrite { message } => {
            let message = match is_error {
                true => message
                    .as_deref()
                    .map(|c| truncate_string(c, MAX_INLINE_CHARS)),
                false => Some("todos updated".to_string()),
            };
            render_todowrite_result(ctx, is_error, message.as_deref())
        }
        ToolResultVariant::NotebookEdit {
            notebook_path,
            cell_identifier,
            cell_type,
            edit_mode,
            diff_content,
        } => format_notebook_result(
            ctx,
            &NotebookEditParams {
                notebook_path,
                cell_identifier,
                cell_type: cell_type.as_deref(),
                edit_mode,
                diff_content: diff_content.as_deref(),
            },
        ),
        ToolResultVariant::ToolError { content } => {
            format_tool_error(ctx, &block.tool_name, content.as_deref())
        }
        ToolResultVariant::Default { content } => {
            let shown = content
                .as_deref()
                .map(|c| truncate_string(c, MAX_INLINE_CHARS));
            render_default_result(ctx, is_error, shown.as_deref())
        }
    }
}
