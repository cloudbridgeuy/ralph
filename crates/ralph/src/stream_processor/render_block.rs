use super::output_block::{ToolInvocationBlock, ToolInvocationVariant};
use super::utils::truncate_string;
use crate::render::{
    render_bash_invocation, render_default_invocation, render_glob_invocation,
    render_grep_invocation, render_read_invocation, render_todowrite_invocation,
    GrepInvocationParams, RenderContext, TodoDisplayItem,
};

const MAX_ARGUMENT_CHARS: usize = 60;

pub fn render_invocation(ctx: &RenderContext, block: &ToolInvocationBlock) -> String {
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
