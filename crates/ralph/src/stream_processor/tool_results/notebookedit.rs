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

use crate::ansi::RESET;
use crate::render::code_frame::framed_diff;
use crate::render::NotebookEditParams;
use crate::render::RenderContext;
use ralph_core::code_frame::Label;

/// Maximum lines to show in a diff before truncating.
const MAX_DIFF_LINES: usize = 50;

pub fn format_notebook_result(ctx: &RenderContext, params: &NotebookEditParams) -> String {
    if let Some(diff_content) = params.diff_content {
        return format_diff_output(ctx, params, diff_content);
    }
    let cell_display = format!("{}:{}", params.notebook_path, params.cell_identifier);
    if ctx.terminal {
        format!(
            "{}\u{2713}{RESET} {}NotebookEdit: {} (no changes){RESET}\n",
            ctx.tools.ok_fg(),
            ctx.tools.label_fg(),
            cell_display
        )
    } else {
        format!("  NotebookEdit: {} (no changes)\n", cell_display)
    }
}

fn format_diff_output(
    ctx: &RenderContext,
    params: &NotebookEditParams,
    diff_content: &str,
) -> String {
    let lines: Vec<&str> = diff_content.lines().collect();
    let line_count = lines.len();

    let (display_content, truncated) = if line_count > MAX_DIFF_LINES {
        let truncated_lines: String = lines[..MAX_DIFF_LINES].join("\n");
        (truncated_lines, true)
    } else {
        (diff_content.to_string(), false)
    };

    let cell_display = format!("{}:{}", params.notebook_path, params.cell_identifier);

    let cell_type_indicator = match params.cell_type {
        Some("code") => " (code)",
        Some("markdown") => " (markdown)",
        _ => "",
    };

    let edit_mode_indicator = match params.edit_mode {
        "insert" => " (new cell)",
        "delete" => " (deleted)",
        _ => "",
    };

    if ctx.terminal {
        let title = format!("{cell_display}{cell_type_indicator}{edit_mode_indicator}");
        let ink = ctx.tools.name_fg();
        let head = Label::File {
            path: &title,
            kind: "diff",
            ink: &ink,
        };
        let frame = framed_diff(ctx, &head, &display_content);
        let mut output = if edit_mode_indicator.is_empty() {
            frame
        } else {
            let accented = format!("{}{edit_mode_indicator}{ink}", ctx.tools.accent_fg());
            frame.replacen(edit_mode_indicator, &accented, 1)
        };

        if truncated {
            output.push_str(&format!(
                "{}... {} more lines{RESET}\n",
                ctx.tools.label_fg(),
                line_count - MAX_DIFF_LINES
            ));
        }

        output
    } else {
        let mut output = String::new();

        if !edit_mode_indicator.is_empty() {
            output.push_str(&format!(
                "-- {}{}{} --\n",
                cell_display, cell_type_indicator, edit_mode_indicator
            ));
        } else {
            output.push_str(&format!("-- {}{} --\n", cell_display, cell_type_indicator));
        }

        output.push_str("```diff\n");
        output.push_str(&display_content);
        if !display_content.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("```\n");

        if truncated {
            output.push_str(&format!("... {} more lines\n", line_count - MAX_DIFF_LINES));
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream_processor::StreamProcessor;
    use ralph_core::stream::blocks::NotebookSnapshot;

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

        let output = format_diff_output(&processor.render_context(), &params(&snapshot), diff);

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

        let output = format_diff_output(&processor.render_context(), &params(&snapshot), diff);

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

        let output = format_diff_output(&processor.render_context(), &params(&snapshot), diff);

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

        let output =
            format_diff_output(&processor.render_context(), &params(&snapshot), &long_diff);

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

        let output =
            format_diff_output(&processor.render_context(), &params(&snapshot), short_diff);

        assert!(!output.contains("more lines"));
    }

    fn params(snapshot: &NotebookSnapshot) -> NotebookEditParams<'_> {
        NotebookEditParams {
            notebook_path: &snapshot.notebook_path,
            cell_identifier: &snapshot.cell_identifier,
            cell_type: snapshot.cell_type.as_deref(),
            edit_mode: &snapshot.edit_mode,
            diff_content: None,
        }
    }

    fn snapshot() -> NotebookSnapshot {
        NotebookSnapshot {
            notebook_path: "n.ipynb".to_string(),
            cell_identifier: "0".to_string(),
            content: None,
            edit_mode: "replace".to_string(),
            cell_type: None,
        }
    }

    #[test]
    fn diff_uses_the_configured_foregrounds() {
        let mut processor = StreamProcessor::with_highlighting(true);
        processor.renderer.diff_colors = crate::diff_theme::test_colors();
        let out = format_diff_output(&processor.render_context(), &params(&snapshot()), "+x\n-y");
        assert!(out.contains("\x1b[94m+x"));
        assert!(out.contains("\x1b[95m-y"));
    }

    #[test]
    fn plain_processor_has_no_escape_byte() {
        let processor = StreamProcessor::with_highlighting(false);
        assert!(!format_diff_output(
            &processor.render_context(),
            &params(&snapshot()),
            "+x\n-y\n@@ -1 @@"
        )
        .contains('\x1b'));
    }

    #[test]
    fn notebook_result_rule_carries_cell() {
        let processor = StreamProcessor::with_highlighting(true);
        let mut inserted = snapshot();
        inserted.edit_mode = "insert".to_string();
        let out = format_diff_output(&processor.render_context(), &params(&inserted), "+x");
        let plain = crate::frame::strip_ansi(&out);
        assert!(
            plain.starts_with("── n.ipynb:0 (new cell) · diff ─"),
            "{plain:?}"
        );
        assert!(!plain.contains("```"), "{plain:?}");
    }

    #[test]
    fn header_uses_the_configured_tool_colors() {
        let mut processor = StreamProcessor::with_highlighting(true);
        processor.renderer.tool_colors = crate::tool_theme::test_colors();
        let mut inserted = snapshot();
        inserted.edit_mode = "insert".to_string();
        let out = format_diff_output(&processor.render_context(), &params(&inserted), "+x");
        assert!(out.starts_with("\x1b[94m\u{2500}\u{2500} n.ipynb:0"));
        assert!(out.contains("\x1b[91m (new cell)"));
    }
}
