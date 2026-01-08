//! NotebookEdit tool result formatting.
//!
//! Formats NotebookEdit tool results with generated diffs and syntax highlighting.
//!
//! Claude CLI's NotebookEdit tool modifies Jupyter notebook cells, returning success
//! messages. To show meaningful diffs, we:
//! 1. Capture cell content before the NotebookEdit tool executes (via NotebookSnapshot)
//! 2. Read the cell again after the edit completes
//! 3. Generate a unified diff between before and after
//! 4. Display with syntax highlighting based on cell type

use std::fs;

use similar::{ChangeTag, TextDiff};

use crate::diff_highlight::highlight_with_basic_colors;

use super::super::processor::StreamProcessor;
use super::super::types::NotebookSnapshot;

/// Maximum lines to show in a diff before truncating.
const MAX_DIFF_LINES: usize = 50;

/// Generate a unified diff from a NotebookSnapshot by comparing before/after cell content.
///
/// Reads the current cell content and generates a unified diff against the
/// captured snapshot. Returns None if the notebook/cell cannot be read or no changes
/// were made.
pub fn generate_diff_from_snapshot(snapshot: &NotebookSnapshot) -> Option<String> {
    // For insert operations with no previous content, we'll show all new content as additions
    // For delete operations, we'll show all previous content as deletions
    let after_content = if snapshot.edit_mode == "delete" {
        // Cell was deleted - after content is empty
        String::new()
    } else {
        // Read current cell content
        read_notebook_cell_content(&snapshot.notebook_path, &snapshot.cell_identifier)?
    };

    // Get before content (empty string if cell didn't exist / insert operation)
    let before_content = snapshot.content.as_deref().unwrap_or("");

    // Skip if no changes
    if before_content == after_content {
        return None;
    }

    // Generate unified diff
    let diff = TextDiff::from_lines(before_content, &after_content);
    let cell_display = format!("{}:{}", snapshot.notebook_path, snapshot.cell_identifier);
    let unified = generate_unified_diff(&diff, &cell_display);

    Some(unified)
}

/// Generate unified diff format from a TextDiff.
fn generate_unified_diff<'a>(diff: &TextDiff<'a, 'a, 'a, str>, cell_display: &str) -> String {
    let mut output = String::new();

    // Add header
    output.push_str(&format!("--- a/{}\n", cell_display));
    output.push_str(&format!("+++ b/{}\n", cell_display));

    // Generate hunks
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        // Hunk header
        output.push_str(&format!("{}", hunk.header()));

        // Changes
        for change in hunk.iter_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            output.push_str(sign);
            output.push_str(change.value());
            if !change.value().ends_with('\n') {
                output.push('\n');
            }
        }
    }

    output
}

/// Format a NotebookEdit tool result using a generated diff from snapshot.
///
/// This is called when we have captured a snapshot before the edit.
/// It reads the current cell content, generates a diff, and formats it
/// with syntax highlighting.
pub fn format_notebook_result_with_snapshot(
    processor: &StreamProcessor,
    snapshot: NotebookSnapshot,
) -> String {
    // Try to generate diff from snapshot
    if let Some(diff_content) = generate_diff_from_snapshot(&snapshot) {
        format_diff_output(processor, &snapshot, &diff_content)
    } else {
        // No changes or couldn't read cell - show simple success message
        let cell_display = format!("{}:{}", snapshot.notebook_path, snapshot.cell_identifier);
        if processor.highlighting_enabled {
            format!(
                "\x1b[32m\u{2713}\x1b[0m \x1b[90mNotebookEdit: {} (no changes)\x1b[0m\n",
                cell_display
            )
        } else {
            format!("  NotebookEdit: {} (no changes)\n", cell_display)
        }
    }
}

/// Format diff output with optional highlighting and truncation.
///
/// This is the common formatting logic for NotebookEdit tool diffs.
fn format_diff_output(
    processor: &StreamProcessor,
    snapshot: &NotebookSnapshot,
    diff_content: &str,
) -> String {
    // Count lines for potential truncation
    let lines: Vec<&str> = diff_content.lines().collect();
    let line_count = lines.len();

    // Truncate if too long
    let (display_content, truncated) = if line_count > MAX_DIFF_LINES {
        let truncated_lines: String = lines[..MAX_DIFF_LINES].join("\n");
        (truncated_lines, true)
    } else {
        (diff_content.to_string(), false)
    };

    // Build cell display string
    let cell_display = format!("{}:{}", snapshot.notebook_path, snapshot.cell_identifier);

    // Determine cell type indicator
    let cell_type_indicator = match snapshot.cell_type.as_deref() {
        Some("code") => " (code)",
        Some("markdown") => " (markdown)",
        _ => "",
    };

    // Determine edit mode indicator
    let edit_mode_indicator = match snapshot.edit_mode.as_str() {
        "insert" => " (new cell)",
        "delete" => " (deleted)",
        _ => "",
    };

    if processor.highlighting_enabled {
        // Highlight the diff
        let highlighted_diff = highlight_with_basic_colors(&display_content);

        // Build output with header
        let mut output = String::new();

        // Cell path header with box drawing and indicators
        if !edit_mode_indicator.is_empty() {
            output.push_str(&format!(
                "\x1b[36m\u{2500}\u{2500} {}{}\x1b[33m{}\x1b[36m \u{2500}\u{2500}\x1b[0m\n",
                cell_display, cell_type_indicator, edit_mode_indicator
            ));
        } else {
            output.push_str(&format!(
                "\x1b[36m\u{2500}\u{2500} {}{} \u{2500}\u{2500}\x1b[0m\n",
                cell_display, cell_type_indicator
            ));
        }

        // The highlighted diff content wrapped in diff fences
        output.push_str("```diff\n");
        output.push_str(&highlighted_diff);
        if !highlighted_diff.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("```\n");

        // Truncation indicator
        if truncated {
            output.push_str(&format!(
                "\x1b[90m... {} more lines\x1b[0m\n",
                line_count - MAX_DIFF_LINES
            ));
        }

        output
    } else {
        // Plain text format
        let mut output = String::new();

        // Simple header with indicators
        if !edit_mode_indicator.is_empty() {
            output.push_str(&format!(
                "-- {}{}{} --\n",
                cell_display, cell_type_indicator, edit_mode_indicator
            ));
        } else {
            output.push_str(&format!("-- {}{} --\n", cell_display, cell_type_indicator));
        }

        // Plain diff content
        output.push_str("```diff\n");
        output.push_str(&display_content);
        if !display_content.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("```\n");

        // Truncation indicator
        if truncated {
            output.push_str(&format!("... {} more lines\n", line_count - MAX_DIFF_LINES));
        }

        output
    }
}

/// Read the content of a specific cell from a Jupyter notebook.
///
/// Returns the cell content as a single string (joining source lines),
/// or None if the notebook or cell cannot be read.
fn read_notebook_cell_content(notebook_path: &str, cell_identifier: &str) -> Option<String> {
    // Read and parse the notebook JSON
    let notebook_content = fs::read_to_string(notebook_path).ok()?;
    let notebook: serde_json::Value = serde_json::from_str(&notebook_content).ok()?;

    // Get the cells array
    let cells = notebook.get("cells")?.as_array()?;

    // Try to find the cell by identifier
    // First try to match by cell_id (string match)
    // Then try to match by index (if identifier is numeric)
    let cell = if let Ok(index) = cell_identifier.parse::<usize>() {
        // Numeric identifier - use as 0-based index
        cells.get(index)
    } else {
        // String identifier - try to match against cell id metadata
        cells.iter().find(|cell| {
            cell.get("id")
                .and_then(|id| id.as_str())
                .map(|id| id == cell_identifier)
                .unwrap_or(false)
        })
    }?;

    // Extract the source content
    // The "source" field can be either a string or an array of strings
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
        let before = "print('hello')\n";
        let after = "print('world')\n";

        let diff = TextDiff::from_lines(before, after);
        let unified = generate_unified_diff(&diff, "notebook.ipynb:0");

        assert!(unified.contains("--- a/notebook.ipynb:0"));
        assert!(unified.contains("+++ b/notebook.ipynb:0"));
        assert!(unified.contains("-print('hello')"));
        assert!(unified.contains("+print('world')"));
    }

    #[test]
    fn test_generate_unified_diff_new_cell() {
        let before = "";
        let after = "# New cell\nprint('hello')\n";

        let diff = TextDiff::from_lines(before, after);
        let unified = generate_unified_diff(&diff, "notebook.ipynb:1");

        assert!(unified.contains("+# New cell"));
        assert!(unified.contains("+print('hello')"));
        // The diff header contains "--- a/" but no deletion lines should exist
        // (deletion lines start with "-" followed by actual content, not header markers)
        assert!(!unified.contains("-# New cell"));
        assert!(!unified.contains("-print"));
    }

    #[test]
    fn test_format_diff_output_code_cell() {
        let processor = StreamProcessor::with_highlighting(false);
        let snapshot = NotebookSnapshot {
            notebook_path: "test.ipynb".to_string(),
            cell_identifier: "0".to_string(),
            content: Some("old\n".to_string()),
            edit_mode: "replace".to_string(),
            cell_type: Some("code".to_string()),
        };
        let diff = "-old\n+new\n";

        let output = format_diff_output(&processor, &snapshot, diff);

        assert!(output.contains("test.ipynb:0"));
        assert!(output.contains("(code)"));
        assert!(!output.contains("(new cell)"));
    }

    #[test]
    fn test_format_diff_output_insert_mode() {
        let processor = StreamProcessor::with_highlighting(false);
        let snapshot = NotebookSnapshot {
            notebook_path: "test.ipynb".to_string(),
            cell_identifier: "1".to_string(),
            content: None,
            edit_mode: "insert".to_string(),
            cell_type: Some("code".to_string()),
        };
        let diff = "+new content\n";

        let output = format_diff_output(&processor, &snapshot, diff);

        assert!(output.contains("(new cell)"));
    }

    #[test]
    fn test_format_diff_output_delete_mode() {
        let processor = StreamProcessor::with_highlighting(false);
        let snapshot = NotebookSnapshot {
            notebook_path: "test.ipynb".to_string(),
            cell_identifier: "2".to_string(),
            content: Some("deleted content\n".to_string()),
            edit_mode: "delete".to_string(),
            cell_type: Some("markdown".to_string()),
        };
        let diff = "-deleted content\n";

        let output = format_diff_output(&processor, &snapshot, diff);

        assert!(output.contains("(deleted)"));
        assert!(output.contains("(markdown)"));
    }

    #[test]
    fn test_format_diff_output_truncation() {
        let processor = StreamProcessor::with_highlighting(false);
        let snapshot = NotebookSnapshot {
            notebook_path: "test.ipynb".to_string(),
            cell_identifier: "0".to_string(),
            content: None,
            edit_mode: "replace".to_string(),
            cell_type: None,
        };
        let long_diff: String = (0..100).map(|i| format!("+line{}\n", i)).collect();

        let output = format_diff_output(&processor, &snapshot, &long_diff);

        assert!(output.contains("... 50 more lines"));
    }

    #[test]
    fn test_format_diff_output_no_truncation() {
        let processor = StreamProcessor::with_highlighting(false);
        let snapshot = NotebookSnapshot {
            notebook_path: "test.ipynb".to_string(),
            cell_identifier: "0".to_string(),
            content: None,
            edit_mode: "replace".to_string(),
            cell_type: None,
        };
        let short_diff = "+line1\n+line2\n";

        let output = format_diff_output(&processor, &snapshot, short_diff);

        assert!(!output.contains("more lines"));
    }
}
