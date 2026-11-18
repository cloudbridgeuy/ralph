mod block_builders;
mod diff;
mod output_block;
mod types;
mod utils;

#[cfg(test)]
mod test_helpers;

pub use block_builders::{build_tool_invocation_block, input_str, write_result_variant};
pub use diff::{edit_diff, notebook_diff};
pub use output_block::{
    extract_response_text, GrepInvocationBuilder, OutputBlock, TextBlock, TodoItem,
    ToolInvocationBlock, ToolInvocationVariant, ToolResultBlock, ToolResultVariant,
};
pub use types::{
    EditSnapshot, GrepParams, KeyArgument, NotebookSnapshot, StreamProcessorResult,
    VerboseToolsConfig, WriteSnapshot,
};
pub use utils::{
    count_non_empty_lines, extract_key_argument, is_content_truncated, truncate_multiline,
    truncate_string,
};
