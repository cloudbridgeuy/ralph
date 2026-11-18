use ralph_core::stream::ToolInvocation;

use super::output_block::{
    GrepInvocationBuilder, OutputBlock, TodoItem, ToolInvocationVariant, ToolResultVariant,
};
use super::types::{EditSnapshot, GrepParams, VerboseToolsConfig, WriteSnapshot};
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

pub fn write_result_variant(snapshot: WriteSnapshot, after: Option<String>) -> ToolResultVariant {
    let file_path = snapshot.file_path;
    let before = snapshot.content.unwrap_or_default();
    match after {
        Some(content) if !snapshot.file_existed && !content.is_empty() => {
            ToolResultVariant::WriteNewFile { file_path, content }
        }
        Some(after) if snapshot.file_existed && after != before => {
            ToolResultVariant::WriteOverwrite {
                file_path,
                before_content: before,
                after_content: after,
            }
        }
        _ => ToolResultVariant::WriteNoChanges {
            file_path,
            is_new_file: !snapshot.file_existed,
        },
    }
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
            write_result_variant(write_snapshot(None, false), Some("hi".to_string())),
            ToolResultVariant::WriteNewFile {
                file_path: "a.txt".to_string(),
                content: "hi".to_string(),
            },
        );
    }

    #[test]
    fn test_write_block_with_same_content_has_no_changes() {
        assert_eq!(
            write_result_variant(write_snapshot(Some("hi"), true), Some("hi".to_string())),
            ToolResultVariant::WriteNoChanges {
                file_path: "a.txt".to_string(),
                is_new_file: false,
            },
        );
    }

    #[test]
    fn test_write_block_for_overwrite() {
        assert_eq!(
            write_result_variant(write_snapshot(Some("old"), true), Some("new".to_string())),
            ToolResultVariant::WriteOverwrite {
                file_path: "a.txt".to_string(),
                before_content: "old".to_string(),
                after_content: "new".to_string(),
            },
        );
    }
}
