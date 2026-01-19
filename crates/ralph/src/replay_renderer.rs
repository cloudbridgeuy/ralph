//! Replay renderer for OutputBlock serialization.
//!
//! This module provides rendering functions for replaying OutputBlock instances
//! with the same visual output as live execution. It uses the same highlighting
//! and formatting logic to ensure replay output is visually identical.
//!
//! # Design
//!
//! The renderer follows the same patterns as the stream processor's event handler:
//! - Text blocks are rendered with markdown/code/diff highlighting
//! - Tool invocations show the `▶ ToolName` header with arguments
//! - Tool results show success/error indicators with content
//! - Separators emit a single newline

use crate::diff_highlight::highlight_with_basic_colors;
use crate::highlight::Highlighter;
use crate::stream_processor::{
    OutputBlock, TextBlock, ToolInvocationBlock, ToolInvocationVariant, ToolResultBlock,
    ToolResultVariant,
};
use ralph_core::chunk::ChunkType;
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
    fn render_text(&self, block: &TextBlock) -> String {
        match &block.chunk.chunk_type {
            ChunkType::Prose => {
                if self.highlighting_enabled {
                    self.markdown_skin
                        .term_text(&block.chunk.content)
                        .to_string()
                } else {
                    block.chunk.content.clone()
                }
            }
            ChunkType::Code { language } => {
                let opening_fence = match language {
                    Some(lang) if !lang.is_empty() => format!("```{}", lang),
                    _ => "```".to_string(),
                };

                let highlighted_content = if self.highlighting_enabled {
                    let lang_ref = language.as_deref();
                    self.code_highlighter
                        .highlight(&block.chunk.content, lang_ref)
                } else {
                    block.chunk.content.clone()
                };

                format!("{}\n{}\n```\n", opening_fence, highlighted_content)
            }
            ChunkType::Diff => {
                let highlighted_content = if self.highlighting_enabled {
                    highlight_with_basic_colors(&block.chunk.content)
                } else {
                    block.chunk.content.clone()
                };

                format!("```diff\n{}\n```\n", highlighted_content)
            }
        }
    }

    /// Render a tool invocation block.
    fn render_tool_invocation(&self, block: &ToolInvocationBlock) -> String {
        match &block.variant {
            ToolInvocationVariant::Bash { command, .. } => self.render_bash_invocation(command),
            grep @ ToolInvocationVariant::Grep { .. } => self.render_grep_invocation(grep),
            ToolInvocationVariant::Read {
                file_path,
                offset,
                limit,
            } => self.render_read_invocation(file_path, *offset, *limit),
            ToolInvocationVariant::Glob { pattern, path } => {
                self.render_glob_invocation(pattern, path.as_deref())
            }
            ToolInvocationVariant::TodoWrite { todos } => self.render_todowrite_invocation(todos),
            ToolInvocationVariant::Default { key_argument, .. } => {
                self.render_default_invocation(&block.tool_name, key_argument.as_deref())
            }
        }
    }

    /// Render Bash tool invocation.
    fn render_bash_invocation(&self, command: &str) -> String {
        let is_multiline = command.contains('\n');

        if self.highlighting_enabled {
            let mut output = String::new();
            output.push_str("\x1b[36m▶ Bash\x1b[0m\n");

            if is_multiline {
                output.push_str("```sh\n");
                let highlighted = self.code_highlighter.highlight(command, Some("sh"));
                output.push_str(&highlighted);
                if !highlighted.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("```\n");
            } else {
                output.push_str("  ");
                let highlighted = self.code_highlighter.highlight(command, Some("sh"));
                let trimmed = highlighted.trim_end_matches("\x1b[0m");
                output.push_str(trimmed);
                output.push_str("\x1b[0m\n");
            }

            output
        } else if is_multiline {
            let mut output = String::new();
            output.push_str("> Bash\n```sh\n");
            output.push_str(command);
            if !command.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("```\n");
            output
        } else {
            format!("> Bash\n  {}\n", command)
        }
    }

    /// Render Grep tool invocation (verbose mode).
    fn render_grep_invocation(&self, variant: &ToolInvocationVariant) -> String {
        let ToolInvocationVariant::Grep {
            pattern,
            path,
            output_mode,
            glob,
            file_type,
            case_insensitive,
        } = variant
        else {
            return String::new();
        };

        let mut output = String::new();

        if self.highlighting_enabled {
            output.push_str("\x1b[36m▶ Grep\x1b[0m\n");

            // Pattern with regex highlighting
            output.push_str("  pattern: ");
            let highlighted = self.code_highlighter.highlight(pattern, Some("regex"));
            output.push_str(&highlighted);
            output.push('\n');

            // Path
            if let Some(p) = path {
                output.push_str(&format!("  path: \x1b[90m{}\x1b[0m\n", p));
            }

            // Options
            if let Some(mode) = output_mode {
                output.push_str(&format!("  mode: \x1b[90m{}\x1b[0m\n", mode));
            }
            if let Some(g) = glob {
                output.push_str(&format!("  glob: \x1b[90m{}\x1b[0m\n", g));
            }
            if let Some(ft) = file_type {
                output.push_str(&format!("  type: \x1b[90m{}\x1b[0m\n", ft));
            }
            if *case_insensitive {
                output.push_str("  case-insensitive: \x1b[90mtrue\x1b[0m\n");
            }
        } else {
            output.push_str("> Grep\n");
            output.push_str(&format!("  pattern: {}\n", pattern));
            if let Some(p) = path {
                output.push_str(&format!("  path: {}\n", p));
            }
            if let Some(mode) = output_mode {
                output.push_str(&format!("  mode: {}\n", mode));
            }
            if let Some(g) = glob {
                output.push_str(&format!("  glob: {}\n", g));
            }
            if let Some(ft) = file_type {
                output.push_str(&format!("  type: {}\n", ft));
            }
            if *case_insensitive {
                output.push_str("  case-insensitive: true\n");
            }
        }

        output
    }

    /// Render Read tool invocation (verbose mode).
    fn render_read_invocation(
        &self,
        file_path: &str,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> String {
        let mut output = String::new();

        if self.highlighting_enabled {
            output.push_str("\x1b[36m▶ Read\x1b[0m\n");
            output.push_str(&format!("  \x1b[90m{}\x1b[0m\n", file_path));

            if let Some(off) = offset {
                output.push_str(&format!("  offset: \x1b[90m{}\x1b[0m\n", off));
            }
            if let Some(lim) = limit {
                output.push_str(&format!("  limit: \x1b[90m{}\x1b[0m\n", lim));
            }
        } else {
            output.push_str("> Read\n");
            output.push_str(&format!("  {}\n", file_path));
            if let Some(off) = offset {
                output.push_str(&format!("  offset: {}\n", off));
            }
            if let Some(lim) = limit {
                output.push_str(&format!("  limit: {}\n", lim));
            }
        }

        output
    }

    /// Render Glob tool invocation (verbose mode).
    fn render_glob_invocation(&self, pattern: &str, path: Option<&str>) -> String {
        if self.highlighting_enabled {
            let mut output = String::new();
            output.push_str("\x1b[36m▶ Glob\x1b[0m\n");
            output.push_str(&format!("  \x1b[90m{}\x1b[0m\n", pattern));
            if let Some(p) = path {
                output.push_str(&format!("  in: \x1b[90m{}\x1b[0m\n", p));
            }
            output
        } else {
            let mut output = String::new();
            output.push_str("> Glob\n");
            output.push_str(&format!("  {}\n", pattern));
            if let Some(p) = path {
                output.push_str(&format!("  in: {}\n", p));
            }
            output
        }
    }

    /// Render TodoWrite tool invocation (verbose mode).
    fn render_todowrite_invocation(&self, todos: &[crate::stream_processor::TodoItem]) -> String {
        let mut output = String::new();

        if self.highlighting_enabled {
            output.push_str("\x1b[36m▶ TodoWrite\x1b[0m\n");

            for todo in todos {
                let status_icon = match todo.status.as_str() {
                    "completed" => "\x1b[32m✓\x1b[0m",
                    "in_progress" => "\x1b[33m⋯\x1b[0m",
                    _ => "\x1b[90m○\x1b[0m",
                };
                output.push_str(&format!("  {} {}\n", status_icon, todo.content));
            }
        } else {
            output.push_str("> TodoWrite\n");

            for todo in todos {
                let status_marker = match todo.status.as_str() {
                    "completed" => "[x]",
                    "in_progress" => "[~]",
                    _ => "[ ]",
                };
                output.push_str(&format!("  {} {}\n", status_marker, todo.content));
            }
        }

        output
    }

    /// Render default tool invocation.
    fn render_default_invocation(&self, tool_name: &str, key_argument: Option<&str>) -> String {
        if self.highlighting_enabled {
            format!(
                "\x1b[36m▶ {}\x1b[0m{}\n",
                tool_name,
                if let Some(arg) = key_argument {
                    format!(" \x1b[90m{}\x1b[0m", arg)
                } else {
                    String::new()
                }
            )
        } else {
            format!("> {} {}\n", tool_name, key_argument.unwrap_or_default())
        }
    }

    /// Render a tool result block.
    fn render_tool_result(&self, block: &ToolResultBlock) -> String {
        match &block.variant {
            ToolResultVariant::Bash { content, truncated } => {
                self.render_bash_result(block.is_error, content.as_deref(), *truncated)
            }
            ToolResultVariant::EditBeforeAfter {
                file_path,
                old_content,
                new_content,
            } => self.render_edit_before_after(file_path, old_content, new_content),
            ToolResultVariant::EditDiff {
                file_path,
                diff_content,
            } => self.render_edit_diff(file_path, diff_content),
            ToolResultVariant::EditNoChanges { file_path } => {
                self.render_no_changes_message(file_path, "edit")
            }
            ToolResultVariant::WriteNewFile { file_path, content } => {
                self.render_write_new_file(file_path, content)
            }
            ToolResultVariant::WriteOverwrite {
                file_path,
                before_content,
                after_content,
            } => self.render_write_overwrite(file_path, before_content, after_content),
            ToolResultVariant::WriteNoChanges {
                file_path,
                is_new_file,
            } => self.render_write_no_changes(file_path, *is_new_file),
            ToolResultVariant::Read {
                file_path,
                content,
                line_count,
                truncated,
            } => self.render_read_result(file_path, content, *line_count, *truncated),
            ToolResultVariant::Grep {
                match_count,
                output_mode,
                content,
            } => self.render_grep_result(*match_count, output_mode, content),
            ToolResultVariant::Glob {
                file_count,
                content,
                truncated,
            } => self.render_glob_result(*file_count, content, *truncated),
            ToolResultVariant::TodoWrite { message } => {
                self.render_todowrite_result(block.is_error, message.as_deref())
            }
            notebook @ ToolResultVariant::NotebookEdit { .. } => {
                self.render_notebook_edit(notebook)
            }
            ToolResultVariant::Default { content } => {
                self.render_default_result(block.is_error, content.as_deref())
            }
        }
    }

    /// Render Bash result.
    fn render_bash_result(&self, is_error: bool, content: Option<&str>, truncated: bool) -> String {
        if self.highlighting_enabled {
            if is_error {
                let mut output = "\x1b[31m✗ Error\x1b[0m\n".to_string();
                if let Some(c) = content {
                    output.push_str(&format!("\x1b[90m{}\x1b[0m\n", c));
                }
                output
            } else if let Some(c) = content {
                let mut output = format!("\x1b[90m{}\x1b[0m\n", c);
                if truncated {
                    output.push_str("\x1b[90m(output truncated)\x1b[0m\n");
                }
                output
            } else {
                "\x1b[32m✓\x1b[0m\n".to_string()
            }
        } else if is_error {
            let mut output = "! Error\n".to_string();
            if let Some(c) = content {
                output.push_str(c);
                output.push('\n');
            }
            output
        } else if let Some(c) = content {
            let mut output = c.to_string();
            output.push('\n');
            if truncated {
                output.push_str("(output truncated)\n");
            }
            output
        } else {
            "OK\n".to_string()
        }
    }

    /// Render Edit before/after result.
    fn render_edit_before_after(
        &self,
        file_path: &str,
        old_content: &str,
        new_content: &str,
    ) -> String {
        let mut output = String::new();

        // File header
        if self.highlighting_enabled {
            output.push_str(&format!("\x1b[90m{}\x1b[0m\n", file_path));
        } else {
            output.push_str(&format!("{}\n", file_path));
        }

        // Detect language for syntax highlighting
        let language = extract_language_from_path(file_path);

        // Before block
        output.push_str(&self.render_content_block(old_content, language, true));

        // Separator
        if self.highlighting_enabled {
            output.push_str("\x1b[90m────────────────────\x1b[0m\n");
        } else {
            output.push_str("────────────────────\n");
        }

        // After block
        output.push_str(&self.render_content_block(new_content, language, false));

        output
    }

    /// Render a content block with line numbers and background color.
    fn render_content_block(
        &self,
        content: &str,
        language: Option<&str>,
        is_before: bool,
    ) -> String {
        let mut output = String::new();
        let lines: Vec<&str> = content.lines().collect();
        let max_line_width = lines.len().to_string().len().max(3);

        for (i, line) in lines.iter().enumerate() {
            let line_num = i + 1;
            let line_num_str = format!("{:>width$}", line_num, width = max_line_width);

            if self.highlighting_enabled {
                // Line number with background color
                let bg_color = if is_before {
                    "\x1b[48;2;100;40;40m" // Red background
                } else {
                    "\x1b[48;2;40;90;40m" // Green background
                };
                output.push_str(&format!("{}{} \x1b[0m ", bg_color, line_num_str));

                // Content with syntax highlighting
                let highlighted = if let Some(lang) = language {
                    self.code_highlighter.highlight(line, Some(lang))
                } else {
                    line.to_string()
                };
                output.push_str(highlighted.trim_end_matches("\x1b[0m"));
                output.push_str("\x1b[0m\n");
            } else {
                output.push_str(&format!("{} │ {}\n", line_num_str, line));
            }
        }

        output
    }

    /// Render Edit diff result.
    fn render_edit_diff(&self, file_path: &str, diff_content: &str) -> String {
        let mut output = String::new();

        if self.highlighting_enabled {
            output.push_str(&format!("\x1b[90m{}\x1b[0m\n", file_path));
            output.push_str(&highlight_with_basic_colors(diff_content));
        } else {
            output.push_str(&format!("{}\n", file_path));
            output.push_str(diff_content);
        }
        output.push('\n');

        output
    }

    /// Render no changes message.
    fn render_no_changes_message(&self, file_path: &str, tool: &str) -> String {
        if self.highlighting_enabled {
            format!(
                "\x1b[90m{}\x1b[0m\n\x1b[33m⚠ No changes ({})\x1b[0m\n",
                file_path, tool
            )
        } else {
            format!("{}\nNo changes ({})\n", file_path, tool)
        }
    }

    /// Render Write new file result.
    fn render_write_new_file(&self, file_path: &str, content: &str) -> String {
        let mut output = String::new();

        // File header with (new file) indicator
        if self.highlighting_enabled {
            output.push_str(&format!(
                "\x1b[90m{}\x1b[0m \x1b[32m(new file)\x1b[0m\n",
                file_path
            ));
        } else {
            output.push_str(&format!("{} (new file)\n", file_path));
        }

        // Content with green background line numbers
        let language = extract_language_from_path(file_path);
        output.push_str(&self.render_content_block(content, language, false));

        output
    }

    /// Render Write overwrite result.
    fn render_write_overwrite(
        &self,
        file_path: &str,
        before_content: &str,
        after_content: &str,
    ) -> String {
        self.render_edit_before_after(file_path, before_content, after_content)
    }

    /// Render Write no changes result.
    fn render_write_no_changes(&self, file_path: &str, is_new_file: bool) -> String {
        if is_new_file {
            if self.highlighting_enabled {
                format!(
                    "\x1b[90m{}\x1b[0m \x1b[32m(new file)\x1b[0m\n\x1b[33m⚠ Empty file created\x1b[0m\n",
                    file_path
                )
            } else {
                format!("{} (new file)\nEmpty file created\n", file_path)
            }
        } else {
            self.render_no_changes_message(file_path, "write")
        }
    }

    /// Render Read result (verbose mode).
    fn render_read_result(
        &self,
        file_path: &str,
        content: &str,
        line_count: usize,
        truncated: bool,
    ) -> String {
        let mut output = String::new();

        // File header with line count
        if self.highlighting_enabled {
            output.push_str(&format!(
                "\x1b[90m{}\x1b[0m ({} lines)\n",
                file_path, line_count
            ));
        } else {
            output.push_str(&format!("{} ({} lines)\n", file_path, line_count));
        }

        // Content with syntax highlighting
        let language = extract_language_from_path(file_path);
        if self.highlighting_enabled {
            if let Some(lang) = language {
                output.push_str(&self.code_highlighter.highlight(content, Some(lang)));
            } else {
                output.push_str(content);
            }
        } else {
            output.push_str(content);
        }

        if !content.ends_with('\n') {
            output.push('\n');
        }

        if truncated {
            if self.highlighting_enabled {
                output.push_str("\x1b[90m(output truncated)\x1b[0m\n");
            } else {
                output.push_str("(output truncated)\n");
            }
        }

        output
    }

    /// Render Grep result (verbose mode).
    fn render_grep_result(&self, match_count: usize, output_mode: &str, content: &str) -> String {
        let mut output = String::new();

        if self.highlighting_enabled {
            output.push_str(&format!(
                "\x1b[32m{} matches\x1b[0m \x1b[90m({})\x1b[0m\n",
                match_count, output_mode
            ));
            output.push_str(&format!("\x1b[90m{}\x1b[0m\n", content));
        } else {
            output.push_str(&format!("{} matches ({})\n", match_count, output_mode));
            output.push_str(content);
            output.push('\n');
        }

        output
    }

    /// Render Glob result (verbose mode).
    fn render_glob_result(&self, file_count: usize, content: &str, truncated: bool) -> String {
        let mut output = String::new();

        if self.highlighting_enabled {
            output.push_str(&format!("\x1b[32m{} files\x1b[0m\n", file_count));
            output.push_str(&format!("\x1b[90m{}\x1b[0m\n", content));
        } else {
            output.push_str(&format!("{} files\n", file_count));
            output.push_str(content);
            output.push('\n');
        }

        if truncated {
            if self.highlighting_enabled {
                output.push_str("\x1b[90m(output truncated)\x1b[0m\n");
            } else {
                output.push_str("(output truncated)\n");
            }
        }

        output
    }

    /// Render TodoWrite result.
    fn render_todowrite_result(&self, is_error: bool, message: Option<&str>) -> String {
        if self.highlighting_enabled {
            if is_error {
                format!(
                    "\x1b[31m✗\x1b[0m {}\n",
                    message.unwrap_or("Failed to update todos")
                )
            } else {
                format!("\x1b[32m✓\x1b[0m {}\n", message.unwrap_or("Todos updated"))
            }
        } else if is_error {
            format!("! {}\n", message.unwrap_or("Failed to update todos"))
        } else {
            format!("  {}\n", message.unwrap_or("Todos updated"))
        }
    }

    /// Render NotebookEdit result.
    fn render_notebook_edit(&self, variant: &ToolResultVariant) -> String {
        let ToolResultVariant::NotebookEdit {
            notebook_path,
            cell_identifier,
            cell_type,
            edit_mode,
            diff_content,
        } = variant
        else {
            return String::new();
        };

        let mut output = String::new();

        if self.highlighting_enabled {
            output.push_str(&format!(
                "\x1b[90m{}\x1b[0m cell {} ({}) [{}]\n",
                notebook_path,
                cell_identifier,
                cell_type.as_deref().unwrap_or("code"),
                edit_mode
            ));
            output.push_str(&highlight_with_basic_colors(diff_content));
        } else {
            output.push_str(&format!(
                "{} cell {} ({}) [{}]\n",
                notebook_path,
                cell_identifier,
                cell_type.as_deref().unwrap_or("code"),
                edit_mode
            ));
            output.push_str(diff_content);
        }
        output.push('\n');

        output
    }

    /// Render default result.
    fn render_default_result(&self, is_error: bool, content: Option<&str>) -> String {
        let display_content = content.unwrap_or("(no output)");

        if self.highlighting_enabled {
            if is_error {
                format!("\x1b[31m✗ Error:\x1b[0m {}\n", display_content)
            } else {
                format!("\x1b[32m✓\x1b[0m \x1b[90m{}\x1b[0m\n", display_content)
            }
        } else if is_error {
            format!("! Error: {}\n", display_content)
        } else {
            format!("  {}\n", display_content)
        }
    }
}

/// Extract programming language from file path extension.
fn extract_language_from_path(file_path: &str) -> Option<&'static str> {
    let ext = file_path.rsplit('.').next()?;
    match ext {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" => Some("javascript"),
        "ts" => Some("typescript"),
        "tsx" | "jsx" => Some("tsx"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" => Some("cpp"),
        "rb" => Some("ruby"),
        "sh" | "bash" | "zsh" => Some("bash"),
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "md" | "markdown" => Some("markdown"),
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        "sql" => Some("sql"),
        "xml" => Some("xml"),
        _ => None,
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

    #[test]
    fn test_extract_language_from_path() {
        assert_eq!(extract_language_from_path("test.rs"), Some("rust"));
        assert_eq!(extract_language_from_path("test.py"), Some("python"));
        assert_eq!(extract_language_from_path("test.js"), Some("javascript"));
        assert_eq!(extract_language_from_path("test.unknown"), None);
        assert_eq!(extract_language_from_path("no_extension"), None);
    }

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
}
