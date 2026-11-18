use ralph_core::stream::ToolInvocation;

use super::output_block::{
    GrepInvocationBuilder, OutputBlock, TodoItem, ToolInvocationVariant, ToolResultVariant,
};
use super::types::{EditSnapshot, GrepParams, NotebookSnapshot, VerboseToolsConfig, WriteSnapshot};
use super::utils::extract_key_argument;

pub fn input_str<'a>(invocation: Option<&'a ToolInvocation>, key: &str) -> Option<&'a str> {
    invocation?.input.get(key)?.as_str()
}

pub fn build_tool_invocation_block(
    invocation: &ToolInvocation,
    verbose: &VerboseToolsConfig,
) -> OutputBlock {
    let name = invocation.name.as_str();
    let input = Some(invocation);
    let variant = match name {
        "Bash" => ToolInvocationVariant::Bash {
            command: input_str(input, "command").unwrap_or("").to_string(),
            description: input_str(input, "description").map(String::from),
        },
        "Grep" | "Read" | "Glob" | "TodoWrite" if !verbose.is_verbose(name) => {
            default_invocation(invocation)
        }
        "Grep" => grep_invocation(&GrepParams::from_invocation_input(&invocation.input)),
        "Read" => ToolInvocationVariant::Read {
            file_path: input_str(input, "file_path")
                .unwrap_or("(unknown file)")
                .to_string(),
            offset: invocation.input.get("offset").and_then(|v| v.as_u64()),
            limit: invocation.input.get("limit").and_then(|v| v.as_u64()),
        },
        "Glob" => ToolInvocationVariant::Glob {
            pattern: input_str(input, "pattern").unwrap_or("*").to_string(),
            path: input_str(input, "path").map(String::from),
        },
        "TodoWrite" => ToolInvocationVariant::TodoWrite {
            todos: invocation
                .input
                .get("todos")
                .and_then(|v| v.as_array())
                .map(|items| items.iter().map(todo_item).collect())
                .unwrap_or_default(),
        },
        _ => default_invocation(invocation),
    };
    OutputBlock::tool_invocation(name, variant)
}

fn default_invocation(invocation: &ToolInvocation) -> ToolInvocationVariant {
    let key_arg = extract_key_argument(&invocation.name, &invocation.input);
    ToolInvocationVariant::Default {
        is_path: key_arg.as_ref().is_some_and(|a| a.is_path),
        key_argument: key_arg.map(|a| a.value),
    }
}

fn grep_invocation(params: &GrepParams) -> ToolInvocationVariant {
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

fn todo_item(item: &serde_json::Value) -> TodoItem {
    let field = |key: &str| item.get(key).and_then(|v| v.as_str()).map(String::from);
    TodoItem {
        content: field("content").unwrap_or_default(),
        status: field("status").unwrap_or_default(),
        active_form: field("activeForm"),
    }
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
            diff_content: Some(diff_content),
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

        let block = build_tool_invocation_block(&invocation, &VerboseToolsConfig::all());
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

        let block = build_tool_invocation_block(&invocation, &VerboseToolsConfig::all());
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

        let block = build_tool_invocation_block(&invocation, &VerboseToolsConfig::all());
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

        let block = build_tool_invocation_block(&invocation, &VerboseToolsConfig::all());
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

    #[test]
    fn test_build_read_invocation_block() {
        let invocation = create_test_invocation(
            "Read",
            json!({"file_path": "src/main.rs", "offset": 10, "limit": 20}),
        );

        let inv = expect_tool_invocation(build_tool_invocation_block(
            &invocation,
            &VerboseToolsConfig::all(),
        ));

        assert_eq!(
            inv.variant,
            ToolInvocationVariant::Read {
                file_path: "src/main.rs".to_string(),
                offset: Some(10),
                limit: Some(20),
            }
        );
    }

    #[test]
    fn test_build_glob_invocation_block() {
        let invocation =
            create_test_invocation("Glob", json!({"pattern": "**/*.rs", "path": "crates"}));

        let inv = expect_tool_invocation(build_tool_invocation_block(
            &invocation,
            &VerboseToolsConfig::all(),
        ));

        assert_eq!(
            inv.variant,
            ToolInvocationVariant::Glob {
                pattern: "**/*.rs".to_string(),
                path: Some("crates".to_string()),
            }
        );
    }

    fn edit_snapshot(old: &str, new: &str) -> EditSnapshot {
        EditSnapshot {
            file_path: "a.rs".to_string(),
            content: None,
            old_string: Some(old.to_string()),
            new_string: Some(new.to_string()),
        }
    }

    #[test]
    fn test_edit_block_with_equal_strings_has_no_changes() {
        assert_eq!(
            build_edit_before_after_block(&edit_snapshot("x", "x")),
            OutputBlock::tool_result(
                "Edit",
                false,
                ToolResultVariant::EditNoChanges {
                    file_path: "a.rs".to_string(),
                },
            )
        );
    }

    #[test]
    fn test_edit_block_with_changed_strings_shows_before_and_after() {
        assert_eq!(
            build_edit_before_after_block(&edit_snapshot("old", "new")),
            OutputBlock::tool_result(
                "Edit",
                false,
                ToolResultVariant::EditBeforeAfter {
                    file_path: "a.rs".to_string(),
                    old_content: "old".to_string(),
                    new_content: "new".to_string(),
                },
            )
        );
    }

    fn write_snapshot(content: Option<&str>, file_existed: bool) -> WriteSnapshot {
        WriteSnapshot {
            file_path: "a.txt".to_string(),
            content: content.map(String::from),
            file_existed,
        }
    }

    #[test]
    fn test_write_block_for_new_file() {
        assert_eq!(
            build_write_result_block(&write_snapshot(None, false), Some("hi")),
            OutputBlock::tool_result(
                "Write",
                false,
                ToolResultVariant::WriteNewFile {
                    file_path: "a.txt".to_string(),
                    content: "hi".to_string(),
                },
            )
        );
    }

    #[test]
    fn test_write_block_with_same_content_has_no_changes() {
        assert_eq!(
            build_write_result_block(&write_snapshot(Some("hi"), true), Some("hi")),
            OutputBlock::tool_result(
                "Write",
                false,
                ToolResultVariant::WriteNoChanges {
                    file_path: "a.txt".to_string(),
                    is_new_file: false,
                },
            )
        );
    }

    #[test]
    fn test_write_block_for_overwrite() {
        assert_eq!(
            build_write_result_block(&write_snapshot(Some("old"), true), Some("new")),
            OutputBlock::tool_result(
                "Write",
                false,
                ToolResultVariant::WriteOverwrite {
                    file_path: "a.txt".to_string(),
                    before_content: "old".to_string(),
                    after_content: "new".to_string(),
                },
            )
        );
    }

    fn notebook_diff(content: Option<&str>, edit_mode: &str, new_source: &str) -> String {
        let snapshot = NotebookSnapshot {
            notebook_path: "n.ipynb".to_string(),
            cell_identifier: "1".to_string(),
            content: content.map(String::from),
            edit_mode: edit_mode.to_string(),
            cell_type: None,
        };
        match build_notebook_edit_block(&snapshot, new_source) {
            OutputBlock::ToolResult(result) => match result.variant {
                ToolResultVariant::NotebookEdit { diff_content, .. } => {
                    diff_content.unwrap_or_default()
                }
                other => panic!("Expected NotebookEdit variant, got {other:?}"),
            },
            other => panic!("Expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn test_notebook_diff_delete_shows_only_old_lines() {
        assert_eq!(notebook_diff(Some("a\nb"), "delete", "c"), "-a\n-b\n");
    }

    #[test]
    fn test_notebook_diff_insert_shows_only_new_lines() {
        assert_eq!(notebook_diff(Some("a"), "insert", "c\nd"), "+c\n+d\n");
    }

    #[test]
    fn test_notebook_diff_replace_shows_old_then_new() {
        assert_eq!(notebook_diff(Some("a"), "replace", "b"), "-a\n+b\n");
    }

    #[test]
    fn test_notebook_diff_replace_skips_empty_sides() {
        assert_eq!(notebook_diff(None, "replace", "b"), "+b\n");
        assert_eq!(notebook_diff(Some("a"), "replace", ""), "-a\n");
    }
}
