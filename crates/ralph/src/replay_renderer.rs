//! Replay renderer for OutputBlock serialization.
//!
//! This module provides rendering functions for replaying OutputBlock instances
//! with the same visual output as live execution. It uses shared rendering
//! functions to ensure replay output is visually identical to live execution.
//!
//! # Design
//!
//! The renderer delegates to shared rendering functions in the `render` module:
//! - Text blocks are rendered with markdown/code/diff highlighting
//! - Tool invocations use shared render_*_invocation functions
//! - Tool results use shared render_*_result functions
//! - Separators emit a single newline

use crate::highlight::Highlighter;
use crate::render::{
    // Shared invocation renderers
    render_bash_invocation,
    // Shared result renderers
    render_bash_result,
    render_default_invocation,
    render_default_result,
    render_edit_before_after,
    render_edit_diff,
    render_glob_invocation,
    render_glob_result,
    render_grep_invocation,
    render_grep_result,
    render_no_changes_message,
    render_notebook_edit,
    render_read_invocation,
    render_read_result,
    render_text_block,
    render_todowrite_invocation,
    render_todowrite_result,
    render_write_new_file,
    render_write_no_changes,
    // Types
    GrepInvocationParams,
    NotebookEditParams,
    RenderContext,
    TodoDisplayItem,
};
use crate::stream_processor::{
    OutputBlock, ToolInvocationBlock, ToolInvocationVariant, ToolResultBlock, ToolResultVariant,
};
use termimad::MadSkin;

/// Configuration for replay rendering.
pub struct ReplayRenderer {
    /// Code highlighter for syntax highlighting.
    code_highlighter: Highlighter,
    /// Markdown renderer for prose.
    markdown_skin: MadSkin,
    /// Whether to enable ANSI color output.
    highlighting_enabled: bool,
}

impl ReplayRenderer {
    /// Create a new replay renderer with the given highlighter.
    pub fn new(highlighter: Highlighter, is_terminal: bool) -> Self {
        Self {
            code_highlighter: highlighter,
            markdown_skin: MadSkin::default(),
            highlighting_enabled: is_terminal,
        }
    }

    /// Create a RenderContext for use with shared rendering functions.
    fn render_context(&self) -> RenderContext<'_> {
        RenderContext::new(&self.code_highlighter, self.highlighting_enabled)
    }

    /// Render an output block to a string.
    pub fn render(&self, block: &OutputBlock) -> String {
        match block {
            OutputBlock::Text(text_block) => self.render_text(text_block),
            OutputBlock::ToolInvocation(inv_block) => self.render_tool_invocation(inv_block),
            OutputBlock::ToolResult(result_block) => self.render_tool_result(result_block),
            OutputBlock::Separator => "\n".to_string(),
        }
    }

    /// Render a text block (prose, code, or diff).
    fn render_text(&self, block: &crate::stream_processor::TextBlock) -> String {
        let ctx = self.render_context();
        let markdown_skin = if self.highlighting_enabled {
            Some(&self.markdown_skin)
        } else {
            None
        };
        let rendered = render_text_block(&ctx, &block.chunk, markdown_skin);

        // Add trailing newline for code and diff blocks to match original behavior
        match &block.chunk.chunk_type {
            ralph_core::chunk::ChunkType::Code { .. } | ralph_core::chunk::ChunkType::Diff => {
                format!("{}\n", rendered)
            }
            ralph_core::chunk::ChunkType::Prose => rendered,
        }
    }

    /// Render a tool invocation block.
    fn render_tool_invocation(&self, block: &ToolInvocationBlock) -> String {
        let ctx = self.render_context();

        match &block.variant {
            ToolInvocationVariant::Bash { command, .. } => render_bash_invocation(&ctx, command),
            ToolInvocationVariant::Grep {
                pattern,
                path,
                output_mode,
                glob,
                file_type,
                case_insensitive,
            } => {
                let params = GrepInvocationParams {
                    pattern,
                    path: path.as_deref(),
                    output_mode: output_mode.as_deref(),
                    glob: glob.as_deref(),
                    file_type: file_type.as_deref(),
                    case_insensitive: *case_insensitive,
                };
                render_grep_invocation(&ctx, &params)
            }
            ToolInvocationVariant::Read {
                file_path,
                offset,
                limit,
            } => render_read_invocation(&ctx, file_path, *offset, *limit),
            ToolInvocationVariant::Glob { pattern, path } => {
                render_glob_invocation(&ctx, pattern, path.as_deref())
            }
            ToolInvocationVariant::TodoWrite { todos } => {
                let display_items: Vec<TodoDisplayItem> = todos
                    .iter()
                    .map(|t| TodoDisplayItem {
                        content: &t.content,
                        status: &t.status,
                        active_form: t.active_form.as_deref(),
                    })
                    .collect();
                render_todowrite_invocation(&ctx, &display_items)
            }
            ToolInvocationVariant::Default { key_argument, .. } => {
                render_default_invocation(&ctx, &block.tool_name, key_argument.as_deref())
            }
        }
    }

    /// Render a tool result block.
    fn render_tool_result(&self, block: &ToolResultBlock) -> String {
        let ctx = self.render_context();

        match &block.variant {
            ToolResultVariant::Bash { content, truncated } => {
                render_bash_result(&ctx, block.is_error, content.as_deref(), *truncated)
            }
            ToolResultVariant::EditBeforeAfter {
                file_path,
                old_content,
                new_content,
            } => render_edit_before_after(&ctx, file_path, old_content, new_content),
            ToolResultVariant::EditDiff {
                file_path,
                diff_content,
            } => render_edit_diff(&ctx, file_path, diff_content),
            ToolResultVariant::EditNoChanges { file_path } => {
                render_no_changes_message(&ctx, file_path, "edit")
            }
            ToolResultVariant::WriteNewFile { file_path, content } => {
                render_write_new_file(&ctx, file_path, content)
            }
            ToolResultVariant::WriteOverwrite {
                file_path,
                before_content,
                after_content,
            } => render_edit_before_after(&ctx, file_path, before_content, after_content),
            ToolResultVariant::WriteNoChanges {
                file_path,
                is_new_file,
            } => render_write_no_changes(&ctx, file_path, *is_new_file),
            ToolResultVariant::Read {
                file_path,
                content,
                line_count,
                truncated,
            } => render_read_result(&ctx, file_path, content, *line_count, *truncated),
            ToolResultVariant::Grep {
                match_count,
                output_mode,
                content,
            } => render_grep_result(&ctx, *match_count, output_mode, content),
            ToolResultVariant::Glob {
                file_count,
                content,
                truncated,
            } => render_glob_result(&ctx, *file_count, content, *truncated),
            ToolResultVariant::TodoWrite { message } => {
                render_todowrite_result(&ctx, block.is_error, message.as_deref())
            }
            ToolResultVariant::NotebookEdit {
                notebook_path,
                cell_identifier,
                cell_type,
                edit_mode,
                diff_content,
            } => {
                let params = NotebookEditParams {
                    notebook_path,
                    cell_identifier,
                    cell_type: cell_type.as_deref(),
                    edit_mode,
                    diff_content,
                };
                render_notebook_edit(&ctx, &params)
            }
            ToolResultVariant::Default { content } => {
                render_default_result(&ctx, block.is_error, content.as_deref())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::ThemeConfig;
    use crate::stream_processor::{
        OutputBlock, TodoItem, ToolInvocationVariant, ToolResultVariant,
    };
    use ralph_core::chunk::{ChunkType, ParsedChunk};

    fn create_test_renderer(highlighting: bool) -> ReplayRenderer {
        let highlighter = Highlighter::with_config(ThemeConfig::default()).unwrap();
        ReplayRenderer::new(highlighter, highlighting)
    }

    #[test]
    fn test_render_separator() {
        let renderer = create_test_renderer(true);
        let block = OutputBlock::separator();
        assert_eq!(renderer.render(&block), "\n");
    }

    #[test]
    fn test_render_prose_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::text(ParsedChunk {
            chunk_type: ChunkType::Prose,
            content: "Hello, world!".to_string(),
        });
        assert_eq!(renderer.render(&block), "Hello, world!");
    }

    #[test]
    fn test_render_code_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::text(ParsedChunk {
            chunk_type: ChunkType::Code {
                language: Some("rust".to_string()),
            },
            content: "fn main() {}".to_string(),
        });
        let result = renderer.render(&block);
        assert!(result.contains("```rust"));
        assert!(result.contains("fn main() {}"));
        assert!(result.ends_with("```\n"));
    }

    #[test]
    fn test_render_diff_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::text(ParsedChunk {
            chunk_type: ChunkType::Diff,
            content: "+added\n-removed".to_string(),
        });
        let result = renderer.render(&block);
        assert!(result.contains("```diff"));
        assert!(result.contains("+added"));
        assert!(result.contains("-removed"));
    }

    #[test]
    fn test_render_bash_invocation_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_invocation(
            "Bash",
            ToolInvocationVariant::Bash {
                command: "ls -la".to_string(),
                description: None,
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("> Bash"));
        assert!(result.contains("ls -la"));
    }

    #[test]
    fn test_render_bash_invocation_multiline_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_invocation(
            "Bash",
            ToolInvocationVariant::Bash {
                command: "echo 'line1'\necho 'line2'".to_string(),
                description: None,
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("> Bash"));
        assert!(result.contains("```sh"));
        assert!(result.contains("echo 'line1'"));
    }

    #[test]
    fn test_render_bash_result_success_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Bash",
            false,
            ToolResultVariant::Bash {
                content: Some("output".to_string()),
                truncated: false,
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("output"));
    }

    #[test]
    fn test_render_bash_result_error_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Bash",
            true,
            ToolResultVariant::Bash {
                content: Some("error message".to_string()),
                truncated: false,
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("! Error"));
        assert!(result.contains("error message"));
    }

    #[test]
    fn test_render_default_invocation_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_invocation(
            "Write",
            ToolInvocationVariant::Default {
                key_argument: Some("/path/to/file.rs".to_string()),
                is_path: true,
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("> Write"));
        assert!(result.contains("/path/to/file.rs"));
    }

    #[test]
    fn test_render_todowrite_invocation_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_invocation(
            "TodoWrite",
            ToolInvocationVariant::TodoWrite {
                todos: vec![
                    TodoItem {
                        content: "Task 1".to_string(),
                        status: "completed".to_string(),
                        active_form: None,
                    },
                    TodoItem {
                        content: "Task 2".to_string(),
                        status: "in_progress".to_string(),
                        active_form: None,
                    },
                    TodoItem {
                        content: "Task 3".to_string(),
                        status: "pending".to_string(),
                        active_form: None,
                    },
                ],
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("> TodoWrite"));
        assert!(result.contains("[x] Task 1"));
        assert!(result.contains("[~] Task 2"));
        assert!(result.contains("[ ] Task 3"));
    }

    #[test]
    fn test_render_edit_before_after_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Edit",
            false,
            ToolResultVariant::EditBeforeAfter {
                file_path: "test.rs".to_string(),
                old_content: "let x = 1;".to_string(),
                new_content: "let x = 2;".to_string(),
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("test.rs"));
        assert!(result.contains("let x = 1;"));
        assert!(result.contains("let x = 2;"));
        assert!(result.contains("────────────────────"));
    }

    #[test]
    fn test_render_write_new_file_plain() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Write",
            false,
            ToolResultVariant::WriteNewFile {
                file_path: "new.rs".to_string(),
                content: "fn main() {}".to_string(),
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("new.rs (new file)"));
        assert!(result.contains("fn main() {}"));
    }

    // Tests for extract_language_from_path are in crate::render::utils

    #[test]
    fn test_render_with_highlighting_has_ansi() {
        let renderer = create_test_renderer(true);
        let block = OutputBlock::tool_invocation(
            "Bash",
            ToolInvocationVariant::Bash {
                command: "ls".to_string(),
                description: None,
            },
        );
        let result = renderer.render(&block);
        // Should contain ANSI escape codes
        assert!(result.contains("\x1b["));
    }

    #[test]
    fn test_render_without_highlighting_no_ansi() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_invocation(
            "Bash",
            ToolInvocationVariant::Bash {
                command: "ls".to_string(),
                description: None,
            },
        );
        let result = renderer.render(&block);
        // Should NOT contain ANSI escape codes
        assert!(!result.contains("\x1b["));
    }

    #[test]
    fn test_render_glob_result_grouped_by_directory() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Glob",
            false,
            ToolResultVariant::Glob {
                file_count: 3,
                content: "src/main.rs\nsrc/lib.rs\ntests/test.rs".to_string(),
                truncated: false,
            },
        );
        let result = renderer.render(&block);
        // Should group files by directory
        assert!(result.contains("3 files matched"));
        assert!(result.contains("src/"));
        assert!(result.contains("tests/"));
        assert!(result.contains("main.rs"));
        assert!(result.contains("lib.rs"));
        assert!(result.contains("test.rs"));
    }

    #[test]
    fn test_render_glob_result_empty() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Glob",
            false,
            ToolResultVariant::Glob {
                file_count: 0,
                content: "".to_string(),
                truncated: false,
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("(no matches)"));
    }

    #[test]
    fn test_render_read_result_normalizes_cat_n() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Read",
            false,
            ToolResultVariant::Read {
                file_path: "test.rs".to_string(),
                content: "     1\tfn main() {\n     2\t    println!(\"hello\");\n     3\t}"
                    .to_string(),
                line_count: 3,
                truncated: false,
            },
        );
        let result = renderer.render(&block);
        // Should normalize line numbers to pipe format
        assert!(result.contains("3 lines"));
        assert!(result.contains("1 │ fn main() {"));
        assert!(result.contains("2 │     println!(\"hello\");"));
        assert!(result.contains("3 │ }"));
    }

    #[test]
    fn test_render_read_result_empty() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Read",
            false,
            ToolResultVariant::Read {
                file_path: "empty.txt".to_string(),
                content: "".to_string(),
                line_count: 0,
                truncated: false,
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("(empty file)"));
    }

    #[test]
    fn test_render_grep_result_files_mode() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Grep",
            false,
            ToolResultVariant::Grep {
                match_count: 3,
                output_mode: "files_with_matches".to_string(),
                content: "src/main.rs\nsrc/lib.rs\ntests/test.rs".to_string(),
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("3 matches"));
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("src/lib.rs"));
    }

    #[test]
    fn test_render_grep_result_empty() {
        let renderer = create_test_renderer(false);
        let block = OutputBlock::tool_result(
            "Grep",
            false,
            ToolResultVariant::Grep {
                match_count: 0,
                output_mode: "files_with_matches".to_string(),
                content: "".to_string(),
            },
        );
        let result = renderer.render(&block);
        assert!(result.contains("(no matches)"));
    }

    // Tests for group_files_by_directory, normalize_cat_n_format, and highlight_grep_match
    // are in crate::render::utils
}
