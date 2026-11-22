use std::path::Path;

use crate::chunk::is_unfenced_diff;
use crate::stream::{ToolInvocation, ToolResult};

use super::block_builders::{input_str, write_result_variant};
use super::diff::{edit_diff, notebook_diff};
use super::utils::{count_non_empty_lines, is_content_truncated};
use super::{
    EditSnapshot, Files, NotebookSnapshot, OutputBlock, ToolResultVariant, VerboseToolsConfig,
    WriteSnapshot,
};

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
    files: &dyn Files,
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
        Some(snapshot) if !result.is_error => snapshot_variant(snapshot, files),
        _ => plain_variant(verbose, name, result, invocation),
    };
    OutputBlock::tool_result(name, result.is_error, variant)
}

fn snapshot_variant(snapshot: Snapshot, files: &dyn Files) -> ToolResultVariant {
    match snapshot {
        Snapshot::Edit(snap) if snap.old_string.is_some() || snap.new_string.is_some() => {
            ToolResultVariant::EditBeforeAfter {
                old_content: snap.old_string.unwrap_or_default(),
                new_content: snap.new_string.unwrap_or_default(),
                file_path: snap.file_path,
            }
        }
        Snapshot::Edit(snap) => match files
            .read_text(Path::new(&snap.file_path))
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
            let after = files.read_text(Path::new(&snap.file_path));
            write_result_variant(snap, after)
        }
        Snapshot::Notebook(snap) => {
            let notebook_json = (snap.edit_mode != "delete")
                .then(|| files.read_text(Path::new(&snap.notebook_path)))
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
        "mcp__mcptools__execute" => {
            script_result(text).unwrap_or(ToolResultVariant::Default { content })
        }
        _ => ToolResultVariant::Default { content },
    }
}

fn script_result(text: &str) -> Option<ToolResultVariant> {
    let serde_json::Value::Object(fields) = serde_json::from_str(text).ok()? else {
        return None;
    };
    let shown = |value: &serde_json::Value| match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let optional = |key: &str| fields.get(key).filter(|v| !v.is_null()).map(shown);
    let logs = match fields.get("logs") {
        Some(serde_json::Value::Array(items)) => items.iter().map(shown).collect(),
        Some(serde_json::Value::Null) | None => Vec::new(),
        Some(other) => vec![shown(other)],
    };
    let variant = ToolResultVariant::Script {
        logs,
        result: optional("result"),
        error: optional("error"),
        trace_id: optional("traceId"),
    };
    let has_fields = ["logs", "result", "error", "traceId"]
        .iter()
        .any(|key| fields.contains_key(*key));
    has_fields.then_some(variant)
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::create_test_invocation;
    use super::super::ImageFile;
    use super::*;
    use crate::stream::ToolResult;
    use serde_json::json;

    struct NoFiles;

    impl Files for NoFiles {
        fn read_text(&self, _path: &Path) -> Option<String> {
            None
        }

        fn exists(&self, _path: &Path) -> bool {
            false
        }

        fn image(&self, path: &str) -> ImageFile {
            ImageFile {
                path: path.to_string(),
                shown: path.into(),
                bytes: None,
            }
        }
    }

    fn execute_result(content: &str, is_error: bool) -> ToolResultVariant {
        let invocation = create_test_invocation("mcp__mcptools__execute", json!({}));
        let result = ToolResult {
            result_type: None,
            tool_use_id: Some("test-id".to_string()),
            content: Some(content.to_string()),
            is_error,
        };
        match build_result_block(
            &VerboseToolsConfig::default(),
            &result,
            Some(&invocation),
            None,
            &NoFiles,
        ) {
            OutputBlock::ToolResult(block) => block.variant,
            _ => panic!("Expected ToolResult"),
        }
    }

    #[test]
    fn execute_result_splits_the_json_fields() {
        let content = json!({
            "logs": ["a", "b"],
            "result": {"n": 1},
            "error": null,
            "traceId": "t-1"
        })
        .to_string();
        assert_eq!(
            execute_result(&content, false),
            ToolResultVariant::Script {
                logs: vec!["a".to_string(), "b".to_string()],
                result: Some("{\"n\":1}".to_string()),
                error: None,
                trace_id: Some("t-1".to_string()),
            }
        );
    }

    #[test]
    fn execute_result_keeps_string_values_unquoted() {
        let content = json!({"logs": [1, "x"], "result": "done", "error": "boom"}).to_string();
        assert_eq!(
            execute_result(&content, true),
            ToolResultVariant::Script {
                logs: vec!["1".to_string(), "x".to_string()],
                result: Some("done".to_string()),
                error: Some("boom".to_string()),
                trace_id: None,
            }
        );
    }

    #[test]
    fn execute_result_that_is_not_the_json_object_stays_default() {
        for content in ["plain text", "[1,2]", "{}", "{\"other\":1}"] {
            assert_eq!(
                execute_result(content, false),
                ToolResultVariant::Default {
                    content: Some(content.to_string())
                }
            );
        }
    }
}
