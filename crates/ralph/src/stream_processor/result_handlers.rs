use std::fs;

use ralph_core::chunk::is_unfenced_diff;
use ralph_core::stream::{ToolInvocation, ToolResult};

use ralph_core::stream::blocks::{
    count_non_empty_lines, edit_diff, input_str, is_content_truncated, notebook_diff,
    write_result_variant,
};
use ralph_core::stream::blocks::{
    EditSnapshot, NotebookSnapshot, VerboseToolsConfig, WriteSnapshot,
};
use ralph_core::stream::blocks::{OutputBlock, ToolResultVariant};

pub enum Snapshot {
    Edit(EditSnapshot),
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
        Snapshot::Edit(snap) if snap.old_string.is_some() || snap.new_string.is_some() => {
            ToolResultVariant::EditBeforeAfter {
                old_content: snap.old_string.unwrap_or_default(),
                new_content: snap.new_string.unwrap_or_default(),
                file_path: snap.file_path,
            }
        }
        Snapshot::Edit(snap) => match fs::read_to_string(&snap.file_path)
            .ok()
            .and_then(|after| edit_diff(&snap, &after))
        {
            Some(diff_content) => ToolResultVariant::EditDiff {
                file_path: snap.file_path,
                diff_content,
            },
            None => ToolResultVariant::EditNoChanges {
                file_path: snap.file_path,
            },
        },
        Snapshot::Write(snap) => {
            let after = fs::read_to_string(&snap.file_path).ok();
            write_result_variant(snap, after)
        }
        Snapshot::Notebook(snap) => {
            let notebook_json = (snap.edit_mode != "delete")
                .then(|| fs::read_to_string(&snap.notebook_path).ok())
                .flatten();
            ToolResultVariant::NotebookEdit {
                diff_content: notebook_diff(&snap, notebook_json.as_deref()),
                notebook_path: snap.notebook_path,
                cell_identifier: snap.cell_identifier,
                cell_type: snap.cell_type,
                edit_mode: snap.edit_mode,
            }
        }
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
