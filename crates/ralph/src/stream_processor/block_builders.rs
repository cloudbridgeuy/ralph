//! Helper functions to build OutputBlock variants from tool data.
//!
//! These functions extract the same data used for display formatting
//! and structure it as OutputBlock variants for replay serialization.

use ralph_core::stream::ToolInvocation;

use super::output_block::{
    GrepInvocationBuilder, OutputBlock, TodoItem, ToolInvocationVariant, ToolResultVariant,
};
use super::types::{EditSnapshot, NotebookSnapshot, WriteSnapshot};
use super::utils::{extract_key_argument, is_content_truncated};

/// Build an OutputBlock from a tool invocation.
///
/// Extracts the relevant data from the invocation and creates the
/// appropriate ToolInvocationVariant.
pub fn build_tool_invocation_block(invocation: &ToolInvocation) -> OutputBlock {
    let variant = match invocation.name.as_str() {
        "Bash" => {
            let command = invocation
                .input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let description = invocation
                .input
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            ToolInvocationVariant::Bash {
                command,
                description,
            }
        }
        "Grep" => {
            let params = super::types::GrepParams::from_invocation_input(&invocation.input);

            let mut builder = GrepInvocationBuilder::new(&params.pattern);
            if let Some(path) = &params.path {
                builder = builder.path(path);
            }
            if let Some(mode) = &params.output_mode {
                builder = builder.output_mode(mode);
            }
            if let Some(glob) = &params.glob {
                builder = builder.glob(glob);
            }
            if let Some(ft) = &params.file_type {
                builder = builder.file_type(ft);
            }
            if params.case_insensitive {
                builder = builder.case_insensitive(true);
            }

            builder.build()
        }
        "Read" => {
            let file_path = invocation
                .input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let offset = invocation.input.get("offset").and_then(|v| v.as_u64());
            let limit = invocation.input.get("limit").and_then(|v| v.as_u64());
            ToolInvocationVariant::Read {
                file_path,
                offset,
                limit,
            }
        }
        "Glob" => {
            let pattern = invocation
                .input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let path = invocation
                .input
                .get("path")
                .and_then(|v| v.as_str())
                .map(String::from);
            ToolInvocationVariant::Glob { pattern, path }
        }
        "TodoWrite" => {
            let todos = invocation
                .input
                .get("todos")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let content = item.get("content")?.as_str()?.to_string();
                            let status = item.get("status")?.as_str()?.to_string();
                            let active_form = item
                                .get("activeForm")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                            Some(TodoItem {
                                content,
                                status,
                                active_form,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            ToolInvocationVariant::TodoWrite { todos }
        }
        _ => {
            // Default: extract key argument
            let key_arg = extract_key_argument(&invocation.name, &invocation.input);
            ToolInvocationVariant::Default {
                key_argument: key_arg.as_ref().map(|a| a.value.clone()),
                is_path: key_arg.as_ref().map(|a| a.is_path).unwrap_or(false),
            }
        }
    };

    OutputBlock::tool_invocation(&invocation.name, variant)
}

/// Build a default OutputBlock for tool results.
pub fn build_default_result_block(
    tool_name: &str,
    content: Option<&str>,
    is_error: bool,
) -> OutputBlock {
    OutputBlock::tool_result(
        tool_name,
        is_error,
        ToolResultVariant::Default {
            content: content.map(String::from),
        },
    )
}

// =============================================================================
// Specialized result block builders
// =============================================================================

/// Build an OutputBlock for Bash tool results.
pub fn build_bash_result_block(content: Option<&str>, is_error: bool) -> OutputBlock {
    let truncated = content.map(is_content_truncated).unwrap_or(false);

    OutputBlock::tool_result(
        "Bash",
        is_error,
        ToolResultVariant::Bash {
            content: content.map(String::from),
            truncated,
        },
    )
}

/// Build an OutputBlock for Edit tool results using before/after display.
///
/// Requires an EditSnapshot captured before the edit was performed.
/// Returns EditBeforeAfter variant with old_string and new_string,
/// or EditNoChanges if no change occurred.
pub fn build_edit_before_after_block(snapshot: &EditSnapshot) -> OutputBlock {
    let old = snapshot.old_string.clone().unwrap_or_default();
    let new = snapshot.new_string.clone().unwrap_or_default();

    // If both are empty or equal, no changes
    if old == new {
        return OutputBlock::tool_result(
            "Edit",
            false,
            ToolResultVariant::EditNoChanges {
                file_path: snapshot.file_path.clone(),
            },
        );
    }

    OutputBlock::tool_result(
        "Edit",
        false,
        ToolResultVariant::EditBeforeAfter {
            file_path: snapshot.file_path.clone(),
            old_content: old,
            new_content: new,
        },
    )
}

/// Build an OutputBlock for Edit tool results with diff content.
///
/// Used when the result contains inline diff content.
pub fn build_edit_diff_block(file_path: &str, diff_content: &str) -> OutputBlock {
    OutputBlock::tool_result(
        "Edit",
        false,
        ToolResultVariant::EditDiff {
            file_path: file_path.to_string(),
            diff_content: diff_content.to_string(),
        },
    )
}

/// Build the appropriate Write result block based on snapshot state.
///
/// - New file: WriteNewFile variant
/// - Overwrite: WriteOverwrite variant with before/after
/// - No changes: WriteNoChanges variant
pub fn build_write_result_block(
    snapshot: &WriteSnapshot,
    new_content: Option<&str>,
) -> OutputBlock {
    let after = new_content.unwrap_or_default();

    if !snapshot.file_existed {
        // New file
        return OutputBlock::tool_result(
            "Write",
            false,
            ToolResultVariant::WriteNewFile {
                file_path: snapshot.file_path.clone(),
                content: after.to_string(),
            },
        );
    }

    // Overwrite
    let before = snapshot.content.as_deref().unwrap_or_default();
    if before == after {
        return OutputBlock::tool_result(
            "Write",
            false,
            ToolResultVariant::WriteNoChanges {
                file_path: snapshot.file_path.clone(),
                is_new_file: false,
            },
        );
    }

    OutputBlock::tool_result(
        "Write",
        false,
        ToolResultVariant::WriteOverwrite {
            file_path: snapshot.file_path.clone(),
            before_content: before.to_string(),
            after_content: after.to_string(),
        },
    )
}

/// Build an OutputBlock for Read tool results (verbose mode).
pub fn build_read_result_block(
    file_path: &str,
    content: &str,
    line_count: usize,
    truncated: bool,
) -> OutputBlock {
    OutputBlock::tool_result(
        "Read",
        false,
        ToolResultVariant::Read {
            file_path: file_path.to_string(),
            content: content.to_string(),
            line_count,
            truncated,
        },
    )
}

/// Build an OutputBlock for Grep tool results (verbose mode).
pub fn build_grep_result_block(
    match_count: usize,
    output_mode: &str,
    content: &str,
) -> OutputBlock {
    OutputBlock::tool_result(
        "Grep",
        false,
        ToolResultVariant::Grep {
            match_count,
            output_mode: output_mode.to_string(),
            content: content.to_string(),
        },
    )
}

/// Build an OutputBlock for Glob tool results (verbose mode).
pub fn build_glob_result_block(file_count: usize, content: &str, truncated: bool) -> OutputBlock {
    OutputBlock::tool_result(
        "Glob",
        false,
        ToolResultVariant::Glob {
            file_count,
            content: content.to_string(),
            truncated,
        },
    )
}

/// Build an OutputBlock for TodoWrite tool results (verbose mode).
pub fn build_todowrite_result_block(message: Option<&str>) -> OutputBlock {
    OutputBlock::tool_result(
        "TodoWrite",
        false,
        ToolResultVariant::TodoWrite {
            message: message.map(String::from),
        },
    )
}

/// Build an OutputBlock for NotebookEdit tool results.
///
/// Requires a NotebookSnapshot for cell identification and the new source content.
pub fn build_notebook_edit_block(snapshot: &NotebookSnapshot, new_source: &str) -> OutputBlock {
    // Generate diff between old and new content
    let old_content = snapshot.content.as_deref().unwrap_or("");
    let diff_content = generate_simple_diff(old_content, new_source, &snapshot.edit_mode);

    OutputBlock::tool_result(
        "NotebookEdit",
        false,
        ToolResultVariant::NotebookEdit {
            notebook_path: snapshot.notebook_path.clone(),
            cell_identifier: snapshot.cell_identifier.clone(),
            cell_type: snapshot.cell_type.clone(),
            edit_mode: snapshot.edit_mode.clone(),
            diff_content,
        },
    )
}

/// Generate a simple diff representation for notebook edits.
///
/// For replace/insert: shows old content with - prefix, new content with + prefix
/// For delete: shows old content with - prefix only
fn generate_simple_diff(old: &str, new: &str, edit_mode: &str) -> String {
    let mut diff = String::new();

    match edit_mode {
        "delete" => {
            // Delete mode: only show removed lines
            for line in old.lines() {
                diff.push_str(&format!("-{}\n", line));
            }
        }
        "insert" => {
            // Insert mode: only show added lines
            for line in new.lines() {
                diff.push_str(&format!("+{}\n", line));
            }
        }
        _ => {
            // Replace mode: show both old and new
            if !old.is_empty() {
                for line in old.lines() {
                    diff.push_str(&format!("-{}\n", line));
                }
            }
            if !new.is_empty() {
                for line in new.lines() {
                    diff.push_str(&format!("+{}\n", line));
                }
            }
        }
    }

    diff
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_processor::test_helpers::{create_test_invocation, expect_tool_invocation};
    use serde_json::json;

    #[test]
    fn test_build_bash_invocation_block() {
        let invocation = create_test_invocation(
            "Bash",
            json!({
                "command": "ls -la",
                "description": "List files"
            }),
        );

        let block = build_tool_invocation_block(&invocation);
        let inv = expect_tool_invocation(block);

        assert_eq!(inv.tool_name, "Bash");
        match inv.variant {
            ToolInvocationVariant::Bash {
                command,
                description,
            } => {
                assert_eq!(command, "ls -la");
                assert_eq!(description, Some("List files".to_string()));
            }
            _ => panic!("Expected Bash variant"),
        }
    }

    #[test]
    fn test_build_grep_invocation_block() {
        let invocation = create_test_invocation(
            "Grep",
            json!({
                "pattern": "fn main",
                "path": "src/",
                "output_mode": "content",
                "-i": true
            }),
        );

        let block = build_tool_invocation_block(&invocation);
        let inv = expect_tool_invocation(block);

        assert_eq!(inv.tool_name, "Grep");
        match inv.variant {
            ToolInvocationVariant::Grep {
                pattern,
                path,
                output_mode,
                case_insensitive,
                ..
            } => {
                assert_eq!(pattern, "fn main");
                assert_eq!(path, Some("src/".to_string()));
                assert_eq!(output_mode, Some("content".to_string()));
                assert!(case_insensitive);
            }
            _ => panic!("Expected Grep variant"),
        }
    }

    #[test]
    fn test_build_default_invocation_block() {
        let invocation = create_test_invocation(
            "WebFetch",
            json!({
                "url": "https://example.com"
            }),
        );

        let block = build_tool_invocation_block(&invocation);
        let inv = expect_tool_invocation(block);

        assert_eq!(inv.tool_name, "WebFetch");
        match inv.variant {
            ToolInvocationVariant::Default { key_argument, .. } => {
                assert_eq!(key_argument, Some("https://example.com".to_string()));
            }
            _ => panic!("Expected Default variant"),
        }
    }

    #[test]
    fn test_build_todowrite_invocation_block() {
        let invocation = create_test_invocation(
            "TodoWrite",
            json!({
                "todos": [
                    {"content": "Fix bug", "status": "in_progress", "activeForm": "Fixing bug"},
                    {"content": "Write tests", "status": "pending"}
                ]
            }),
        );

        let block = build_tool_invocation_block(&invocation);
        let inv = expect_tool_invocation(block);

        assert_eq!(inv.tool_name, "TodoWrite");
        match inv.variant {
            ToolInvocationVariant::TodoWrite { todos } => {
                assert_eq!(todos.len(), 2);
                assert_eq!(todos[0].content, "Fix bug");
                assert_eq!(todos[0].status, "in_progress");
                assert_eq!(todos[0].active_form, Some("Fixing bug".to_string()));
                assert_eq!(todos[1].content, "Write tests");
                assert_eq!(todos[1].status, "pending");
                assert_eq!(todos[1].active_form, None);
            }
            _ => panic!("Expected TodoWrite variant"),
        }
    }
}
