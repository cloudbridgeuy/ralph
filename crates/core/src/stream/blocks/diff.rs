use serde_json::Value;
use similar::{ChangeTag, TextDiff};

use super::{EditSnapshot, NotebookSnapshot};

pub fn edit_diff(snapshot: &EditSnapshot, after: &str) -> Option<String> {
    let before = snapshot.content.as_deref().unwrap_or("");
    if before == after {
        return None;
    }
    Some(unified_diff(before, after, &snapshot.file_path))
}

pub fn notebook_diff(snapshot: &NotebookSnapshot, notebook_json: Option<&str>) -> Option<String> {
    let after = if snapshot.edit_mode == "delete" {
        String::new()
    } else {
        cell_content(notebook_json?, &snapshot.cell_identifier)?
    };
    let before = snapshot.content.as_deref().unwrap_or("");
    if before == after {
        return None;
    }
    let cell_display = format!("{}:{}", snapshot.notebook_path, snapshot.cell_identifier);
    Some(unified_diff(before, &after, &cell_display))
}

fn unified_diff(before: &str, after: &str, label: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let mut output = format!("--- a/{label}\n+++ b/{label}\n");
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        output.push_str(&format!("{}\n", hunk.header()));
        for change in hunk.iter_changes() {
            output.push_str(match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            });
            output.push_str(change.value());
            if !change.value().ends_with('\n') {
                output.push('\n');
            }
        }
    }
    output
}

fn cell_content(notebook_json: &str, cell_identifier: &str) -> Option<String> {
    let notebook: Value = serde_json::from_str(notebook_json).ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_unified_diff() {
        let unified = unified_diff(
            "line1\nline2\nline3\n",
            "line1\nmodified\nline3\n",
            "test.txt",
        );
        assert!(unified.contains("--- a/test.txt"));
        assert!(unified.contains("+++ b/test.txt"));
        assert!(unified.contains("-line2"));
        assert!(unified.contains("+modified"));
    }

    #[test]
    fn test_generate_unified_diff_addition() {
        let unified = unified_diff("line1\n", "line1\nline2\n", "test.txt");
        assert!(unified.contains("+line2"));
        assert!(!unified.contains("-line1"));
    }

    #[test]
    fn test_generate_unified_diff_deletion() {
        let unified = unified_diff("line1\nline2\n", "line1\n", "test.txt");
        assert!(unified.contains("-line2"));
        assert!(!unified.contains("+line2"));
    }

    #[test]
    fn test_generate_unified_diff_new_cell() {
        let unified = unified_diff("", "# New cell\nprint('hello')\n", "notebook.ipynb:1");
        assert!(unified.contains("+# New cell"));
        assert!(unified.contains("+print('hello')"));
        assert!(!unified.contains("-# New cell"));
        assert!(!unified.contains("-print"));
    }

    #[test]
    fn unified_diff_breaks_line_after_hunk_header() {
        let out = unified_diff("x = 1\n", "x = 2\n", "a.rs");
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.contains(&"@@ -1 +1 @@"));
        assert!(!lines
            .iter()
            .any(|l| l.starts_with("@@") && !l.ends_with("@@")));
    }

    fn notebook_snapshot(edit_mode: &str, content: Option<&str>) -> NotebookSnapshot {
        NotebookSnapshot {
            notebook_path: "n.ipynb".to_string(),
            cell_identifier: "0".to_string(),
            content: content.map(str::to_string),
            edit_mode: edit_mode.to_string(),
            cell_type: None,
        }
    }

    #[test]
    fn notebook_diff_reads_cell_source_array() {
        let json = r#"{"cells":[{"source":["x = 2\n"]}]}"#;
        let out = notebook_diff(&notebook_snapshot("replace", Some("x = 1\n")), Some(json));
        assert!(out.unwrap().contains("+x = 2"));
    }

    #[test]
    fn notebook_diff_is_none_when_unreadable_unless_deleting() {
        assert!(notebook_diff(&notebook_snapshot("replace", Some("a\n")), None).is_none());
        let out = notebook_diff(&notebook_snapshot("delete", Some("a\n")), None);
        assert!(out.unwrap().contains("-a"));
    }
}
