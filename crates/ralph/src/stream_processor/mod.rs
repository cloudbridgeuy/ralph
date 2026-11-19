mod block_renderer;
#[cfg(test)]
mod processor;
mod render_block;
mod tool_results;

pub use block_renderer::BlockRenderer;
#[cfg(test)]
pub use processor::StreamProcessor;
pub use ralph_core::stream::blocks::{
    extract_response_text, OutputBlock, StreamProcessorResult, ToolInvocationBlock,
    ToolInvocationVariant, ToolResultBlock, ToolResultVariant, VerboseToolsConfig,
};

#[cfg(test)]
pub use ralph_core::stream::blocks::{extract_key_argument, truncate_string, KeyArgument};

#[cfg(test)]
mod tests;
