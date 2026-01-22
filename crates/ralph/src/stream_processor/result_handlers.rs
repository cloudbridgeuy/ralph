//! Tool result handling functions (Functional Core).
//!
//! Each function handles a specific tool type's result processing,
//! returning both the formatted output and the output block for replay.

use ralph_core::stream::{ToolInvocation, ToolResult};

use super::block_builders::{
    build_bash_result_block, build_default_result_block, build_edit_before_after_block,
    build_edit_diff_block, build_glob_result_block, build_grep_result_block,
    build_notebook_edit_block, build_read_result_block, build_todowrite_result_block,
    build_write_result_block,
};
use super::output_block::OutputBlock;
use super::processor::StreamProcessor;
use super::tool_results;
use super::types::{EditSnapshot, NotebookSnapshot, WriteSnapshot};
use super::utils::{count_non_empty_lines, is_content_truncated};

/// Result of processing a tool result.
pub struct ToolResultOutput {
    /// Formatted string for terminal display.
    pub formatted: String,
    /// Output block for replay serialization.
    pub block: OutputBlock,
}

/// Handle Edit tool result with snapshot.
pub fn handle_edit_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
    snapshot: Option<&EditSnapshot>,
) -> ToolResultOutput {
    if let Some(snap) = snapshot {
        let has_diff_content = result
            .content
            .as_ref()
            .map(|c| ralph_core::chunk::is_unfenced_diff(c))
            .unwrap_or(false);

        if has_diff_content {
            // Result contains diff - use diff formatting and block
            let formatted = tool_results::format_tool_result_with_context(
                processor,
                result,
                Some(invocation.clone()),
            );
            let block =
                build_edit_diff_block(&snap.file_path, result.content.as_deref().unwrap_or(""));
            ToolResultOutput { formatted, block }
        } else {
            // No diff in result - generate from snapshot
            let formatted = tool_results::format_edit_result_with_snapshot(processor, snap.clone());
            let block = build_edit_before_after_block(snap);
            ToolResultOutput { formatted, block }
        }
    } else {
        handle_default_result(processor, result, Some(invocation))
    }
}

/// Handle Write tool result with snapshot.
pub fn handle_write_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
    snapshot: Option<&WriteSnapshot>,
) -> ToolResultOutput {
    if let Some(snap) = snapshot {
        let formatted = tool_results::format_write_result_with_snapshot(processor, snap.clone());
        // Read the new file content to determine which variant to use
        let new_content = std::fs::read_to_string(&snap.file_path).ok();
        let block = build_write_result_block(snap, new_content.as_deref());
        ToolResultOutput { formatted, block }
    } else {
        handle_default_result(processor, result, Some(invocation))
    }
}

/// Handle NotebookEdit tool result with snapshot.
pub fn handle_notebook_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
    snapshot: Option<&NotebookSnapshot>,
) -> ToolResultOutput {
    if let Some(snap) = snapshot {
        let formatted = tool_results::format_notebook_result_with_snapshot(processor, snap.clone());
        let new_source = invocation
            .input
            .get("new_source")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let block = build_notebook_edit_block(snap, new_source);
        ToolResultOutput { formatted, block }
    } else {
        handle_default_result(processor, result, Some(invocation))
    }
}

/// Handle Bash tool result.
pub fn handle_bash_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: Option<&ToolInvocation>,
) -> ToolResultOutput {
    let formatted =
        tool_results::format_tool_result_with_context(processor, result, invocation.cloned());
    let block = build_bash_result_block(result.content.as_deref(), result.is_error);
    ToolResultOutput { formatted, block }
}

/// Handle Read tool result.
pub fn handle_read_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
) -> ToolResultOutput {
    let formatted =
        tool_results::format_tool_result_with_context(processor, result, Some(invocation.clone()));
    let content = result.content.as_deref().unwrap_or("");
    let line_count = content.lines().count();
    let truncated = is_content_truncated(content);
    let file_path = invocation
        .input
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let block = build_read_result_block(file_path, content, line_count, truncated);
    ToolResultOutput { formatted, block }
}

/// Handle Grep tool result.
pub fn handle_grep_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
) -> ToolResultOutput {
    let formatted =
        tool_results::format_tool_result_with_context(processor, result, Some(invocation.clone()));
    let content = result.content.as_deref().unwrap_or("");
    let match_count = count_non_empty_lines(content);
    let output_mode = invocation
        .input
        .get("output_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("files_with_matches");
    let block = build_grep_result_block(match_count, output_mode, content);
    ToolResultOutput { formatted, block }
}

/// Handle Glob tool result.
pub fn handle_glob_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: &ToolInvocation,
) -> ToolResultOutput {
    let formatted =
        tool_results::format_tool_result_with_context(processor, result, Some(invocation.clone()));
    let content = result.content.as_deref().unwrap_or("");
    let file_count = count_non_empty_lines(content);
    let truncated = is_content_truncated(content);
    let block = build_glob_result_block(file_count, content, truncated);
    ToolResultOutput { formatted, block }
}

/// Handle TodoWrite tool result.
pub fn handle_todowrite_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: Option<&ToolInvocation>,
) -> ToolResultOutput {
    let formatted =
        tool_results::format_tool_result_with_context(processor, result, invocation.cloned());
    let block = build_todowrite_result_block(result.content.as_deref());
    ToolResultOutput { formatted, block }
}

/// Handle default tool result (unknown tools or errors).
pub fn handle_default_result(
    processor: &StreamProcessor,
    result: &ToolResult,
    invocation: Option<&ToolInvocation>,
) -> ToolResultOutput {
    let formatted =
        tool_results::format_tool_result_with_context(processor, result, invocation.cloned());
    let tool_name = invocation.map(|i| i.name.as_str()).unwrap_or("Unknown");
    let block = build_default_result_block(tool_name, result.content.as_deref(), result.is_error);
    ToolResultOutput { formatted, block }
}
