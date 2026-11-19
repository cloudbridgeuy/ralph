mod block_builders;
mod collector;
mod diff;
mod files;
mod output_block;
mod result_handlers;
mod snapshot_capture;
mod types;
mod utils;

#[cfg(test)]
mod test_helpers;

pub use block_builders::input_str;
pub use collector::StreamCollector;
pub use diff::edit_diff;
pub use files::Files;
pub use output_block::{
    extract_response_text, GrepInvocationBuilder, ImageFile, OutputBlock, ToolInvocationBlock,
    ToolInvocationVariant, ToolResultBlock, ToolResultVariant,
};
pub use result_handlers::Snapshot;
pub use types::{
    EditSnapshot, KeyArgument, NotebookSnapshot, StreamProcessorResult, VerboseToolsConfig,
    WriteSnapshot,
};
pub use utils::{extract_key_argument, truncate_multiline, truncate_string};
