use std::path::Path;

use crate::stream::ToolInvocation;

use super::{EditSnapshot, Files, NotebookSnapshot, WriteSnapshot};

fn input_text<'a>(invocation: &'a ToolInvocation, key: &str) -> Option<&'a str> {
    invocation.input.get(key).and_then(|v| v.as_str())
}

pub fn capture_edit_snapshot(
    invocation: &ToolInvocation,
    files: &dyn Files,
) -> Option<EditSnapshot> {
    let file_path = input_text(invocation, "file_path")?;
    Some(EditSnapshot {
        file_path: file_path.to_string(),
        content: files.read_text(Path::new(file_path)),
        old_string: input_text(invocation, "old_string").map(String::from),
        new_string: input_text(invocation, "new_string").map(String::from),
    })
}

pub fn capture_write_snapshot(
    invocation: &ToolInvocation,
    files: &dyn Files,
) -> Option<WriteSnapshot> {
    let file_path = input_text(invocation, "file_path")?;
    let path = Path::new(file_path);
    Some(WriteSnapshot {
        file_path: file_path.to_string(),
        content: files.read_text(path),
        file_existed: files.exists(path),
    })
}

pub fn capture_notebook_snapshot(
    invocation: &ToolInvocation,
    files: &dyn Files,
) -> Option<NotebookSnapshot> {
    let notebook_path = input_text(invocation, "notebook_path")?;
    let cell_identifier = input_text(invocation, "cell_id")
        .map(String::from)
        .or_else(|| {
            invocation
                .input
                .get("cell_number")
                .and_then(|v| v.as_u64())
                .map(|n| n.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let content = read_notebook_cell_content(files, notebook_path, &cell_identifier);
    Some(NotebookSnapshot {
        notebook_path: notebook_path.to_string(),
        cell_identifier,
        content,
        edit_mode: input_text(invocation, "edit_mode")
            .unwrap_or("replace")
            .to_string(),
        cell_type: input_text(invocation, "cell_type").map(String::from),
    })
}

fn read_notebook_cell_content(
    files: &dyn Files,
    notebook_path: &str,
    cell_identifier: &str,
) -> Option<String> {
    let notebook_content = files.read_text(Path::new(notebook_path))?;
    let notebook: serde_json::Value = serde_json::from_str(&notebook_content).ok()?;
    let cells = notebook.get("cells")?.as_array()?;
    let cell = if let Ok(index) = cell_identifier.parse::<usize>() {
        cells.get(index)
    } else {
        cells.iter().find(|cell| {
            cell.get("id")
                .and_then(|id| id.as_str())
                .map(|id| id == cell_identifier)
                .unwrap_or(false)
        })
    }?;
    let source = cell.get("source")?;
    if let Some(s) = source.as_str() {
        Some(s.to_string())
    } else if let Some(arr) = source.as_array() {
        let lines: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        Some(lines.join(""))
    } else {
        None
    }
}
