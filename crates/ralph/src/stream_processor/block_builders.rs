//! Helper functions to build OutputBlock variants from tool data.
//!
//! These functions extract the same data used for display formatting
//! and structure it as OutputBlock variants for replay serialization.

use ralph_core::stream::ToolInvocation;

use super::output_block::{OutputBlock, TodoItem, ToolInvocationVariant, ToolResultVariant};
use super::utils::extract_key_argument;

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
            let output_mode = invocation
                .input
                .get("output_mode")
                .and_then(|v| v.as_str())
                .map(String::from);
            let glob = invocation
                .input
                .get("glob")
                .and_then(|v| v.as_str())
                .map(String::from);
            let file_type = invocation
                .input
                .get("type")
                .and_then(|v| v.as_str())
                .map(String::from);
            let case_insensitive = invocation
                .input
                .get("-i")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            ToolInvocationVariant::Grep {
                pattern,
                path,
                output_mode,
                glob,
                file_type,
                case_insensitive,
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_build_bash_invocation_block() {
        let invocation = ToolInvocation {
            id: "test-id".to_string(),
            name: "Bash".to_string(),
            input: json!({
                "command": "ls -la",
                "description": "List files"
            })
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .collect(),
        };

        let block = build_tool_invocation_block(&invocation);

        match block {
            OutputBlock::ToolInvocation(inv) => {
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
            _ => panic!("Expected ToolInvocation"),
        }
    }

    #[test]
    fn test_build_grep_invocation_block() {
        let invocation = ToolInvocation {
            id: "test-id".to_string(),
            name: "Grep".to_string(),
            input: json!({
                "pattern": "fn main",
                "path": "src/",
                "output_mode": "content",
                "-i": true
            })
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .collect(),
        };

        let block = build_tool_invocation_block(&invocation);

        match block {
            OutputBlock::ToolInvocation(inv) => {
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
            _ => panic!("Expected ToolInvocation"),
        }
    }

    #[test]
    fn test_build_default_invocation_block() {
        let invocation = ToolInvocation {
            id: "test-id".to_string(),
            name: "WebFetch".to_string(),
            input: json!({
                "url": "https://example.com"
            })
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .collect(),
        };

        let block = build_tool_invocation_block(&invocation);

        match block {
            OutputBlock::ToolInvocation(inv) => {
                assert_eq!(inv.tool_name, "WebFetch");
                match inv.variant {
                    ToolInvocationVariant::Default { key_argument, .. } => {
                        assert_eq!(key_argument, Some("https://example.com".to_string()));
                    }
                    _ => panic!("Expected Default variant"),
                }
            }
            _ => panic!("Expected ToolInvocation"),
        }
    }

    #[test]
    fn test_build_todowrite_invocation_block() {
        let invocation = ToolInvocation {
            id: "test-id".to_string(),
            name: "TodoWrite".to_string(),
            input: json!({
                "todos": [
                    {"content": "Fix bug", "status": "in_progress", "activeForm": "Fixing bug"},
                    {"content": "Write tests", "status": "pending"}
                ]
            })
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .collect(),
        };

        let block = build_tool_invocation_block(&invocation);

        match block {
            OutputBlock::ToolInvocation(inv) => {
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
            _ => panic!("Expected ToolInvocation"),
        }
    }
}
