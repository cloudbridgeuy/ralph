//! Tool result handling functions (Functional Core).
//!
//! Each function handles a specific tool type's result processing,
//! returning both the formatted output and the output block.

use std::fs;

use ralph_core::chunk::is_unfenced_diff;
use ralph_core::stream::{ToolInvocation, ToolResult};

use super::block_builders::{
    build_default_result_block, build_edit_before_after_block, input_str, write_result_variant,
};
use super::output_block::{OutputBlock, ToolResultVariant};
use super::processor::StreamProcessor;
use super::tool_results;
use super::tool_results::notebook_diff_from_snapshot;
use super::types::{EditSnapshot, NotebookSnapshot, VerboseToolsConfig, WriteSnapshot};
use super::utils::{count_non_empty_lines, is_content_truncated};

/// Result of processing a tool result.
pub struct ToolResultOutput {
    /// Formatted string for terminal display.
    pub formatted: String,
    /// Output block that captures the result data.
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
        let formatted = tool_results::format_edit_result_with_snapshot(
            &processor.render_context(),
            snap.clone(),
        );
        let block = build_edit_before_after_block(snap);
        ToolResultOutput { formatted, block }
    } else {
        handle_default_result(processor, result, Some(invocation))
    }
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

pub enum Snapshot {
    Write(WriteSnapshot),
    Notebook(NotebookSnapshot),
}

pub fn build_result_block(
    verbose: &VerboseToolsConfig,
    result: &ToolResult,
    invocation: Option<&ToolInvocation>,
    snapshot: Option<Snapshot>,
) -> OutputBlock {
    let name = invocation.map_or("Unknown", |inv| inv.name.as_str());
    let content = result.content.clone();
    let variant = match snapshot {
        _ if name == "Edit"
            && !result.is_error
            && content.as_deref().is_some_and(is_unfenced_diff) =>
        {
            ToolResultVariant::EditDiff {
                file_path: input_str(invocation, "file_path")
                    .unwrap_or("unknown file")
                    .to_string(),
                diff_content: content.unwrap_or_default(),
            }
        }
        Some(snapshot) if !result.is_error => snapshot_variant(snapshot),
        _ => plain_variant(verbose, name, result, invocation),
    };
    OutputBlock::tool_result(name, result.is_error, variant)
}

fn snapshot_variant(snapshot: Snapshot) -> ToolResultVariant {
    match snapshot {
        Snapshot::Write(snap) => {
            let after = fs::read_to_string(&snap.file_path).ok();
            write_result_variant(snap, after)
        }
        Snapshot::Notebook(snap) => ToolResultVariant::NotebookEdit {
            diff_content: notebook_diff_from_snapshot(&snap),
            notebook_path: snap.notebook_path,
            cell_identifier: snap.cell_identifier,
            cell_type: snap.cell_type,
            edit_mode: snap.edit_mode,
        },
    }
}

fn plain_variant(
    verbose: &VerboseToolsConfig,
    name: &str,
    result: &ToolResult,
    invocation: Option<&ToolInvocation>,
) -> ToolResultVariant {
    let content = result.content.clone();
    let text = content.as_deref().unwrap_or("");
    let verbose_tool =
        matches!(name, "Read" | "Grep" | "Glob" | "TodoWrite") && verbose.is_verbose(name);
    match name {
        "Bash" => ToolResultVariant::Bash {
            truncated: is_content_truncated(text),
            content,
        },
        "Read" | "Grep" | "Glob" if verbose_tool && result.is_error => {
            ToolResultVariant::ToolError { content }
        }
        "Read" if verbose_tool => ToolResultVariant::Read {
            file_path: input_str(invocation, "file_path").unwrap_or("").to_string(),
            line_count: text.lines().count(),
            truncated: is_content_truncated(text),
            content: text.to_string(),
        },
        "Grep" if verbose_tool => ToolResultVariant::Grep {
            match_count: count_non_empty_lines(text),
            output_mode: input_str(invocation, "output_mode")
                .unwrap_or("files_with_matches")
                .to_string(),
            content: text.to_string(),
        },
        "Glob" if verbose_tool => ToolResultVariant::Glob {
            file_count: count_non_empty_lines(text),
            truncated: is_content_truncated(text),
            content: text.to_string(),
        },
        "TodoWrite" if verbose_tool => ToolResultVariant::TodoWrite { message: content },
        _ => ToolResultVariant::Default { content },
    }
}
