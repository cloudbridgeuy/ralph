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

use std::collections::BTreeMap;

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
    ///
    /// Normalizes cat-n format and displays with syntax highlighting, matching stream processor.
    fn render_read_result(
        &self,
        file_path: &str,
        content: &str,
        line_count: usize,
        truncated: bool,
    ) -> String {
        const MAX_CONTENT_LINES: usize = 100;

        // Empty result
        if content.is_empty() {
            return if self.highlighting_enabled {
                "\x1b[90m(empty file)\x1b[0m\n".to_string()
            } else {
                "(empty file)\n".to_string()
            };
        }

        // Check for binary file indicator
        if content.contains("(binary file)") || content.starts_with('\u{0}') {
            return if self.highlighting_enabled {
                "\x1b[90m(binary file)\x1b[0m\n".to_string()
            } else {
                "(binary file)\n".to_string()
            };
        }

        // Normalize cat-n format before processing
        let normalized_content = normalize_cat_n_format(content);

        // Count lines for potential truncation
        let lines: Vec<&str> = normalized_content.lines().collect();
        let actual_line_count = lines.len();
        let (display_lines, should_truncate) = if actual_line_count > MAX_CONTENT_LINES {
            (&lines[..MAX_CONTENT_LINES], true)
        } else {
            (&lines[..], truncated)
        };

        let language = extract_language_from_path(file_path);
        let mut output = String::new();

        if self.highlighting_enabled {
            // Results header showing line count
            let line_word = if line_count == 1 { "line" } else { "lines" };
            output.push_str(&format!(
                "\x1b[32m✓\x1b[0m \x1b[90m{} {}\x1b[0m\n",
                line_count, line_word
            ));

            // Apply syntax highlighting to the content
            let content_to_highlight = display_lines.join("\n");
            let highlighted = if language.is_some() {
                self.code_highlighter
                    .highlight(&content_to_highlight, language)
            } else {
                content_to_highlight.clone()
            };

            // Display highlighted content with indentation
            for line in highlighted.lines() {
                output.push_str(&format!("  {}\n", line));
            }

            if should_truncate {
                output.push_str(&format!(
                    "\x1b[90m... {} more lines\x1b[0m\n",
                    actual_line_count.saturating_sub(MAX_CONTENT_LINES)
                ));
            }
        } else {
            // Plain text format
            let line_word = if line_count == 1 { "line" } else { "lines" };
            output.push_str(&format!("{} {}\n", line_count, line_word));

            for line in display_lines {
                output.push_str(&format!("  {}\n", line));
            }

            if should_truncate {
                output.push_str(&format!(
                    "... {} more lines\n",
                    actual_line_count.saturating_sub(MAX_CONTENT_LINES)
                ));
            }
        }

        output
    }

    /// Render Grep result (verbose mode).
    ///
    /// Formats based on output mode with appropriate coloring, matching stream processor.
    fn render_grep_result(&self, match_count: usize, output_mode: &str, content: &str) -> String {
        const MAX_RESULT_LINES: usize = 100;

        // Empty result
        if content.is_empty() {
            return if self.highlighting_enabled {
                "\x1b[90m(no matches)\x1b[0m\n".to_string()
            } else {
                "(no matches)\n".to_string()
            };
        }

        // Count lines for potential truncation
        let lines: Vec<&str> = content.lines().collect();
        let line_count = lines.len();
        let (display_lines, truncated) = if line_count > MAX_RESULT_LINES {
            (&lines[..MAX_RESULT_LINES], true)
        } else {
            (&lines[..], false)
        };

        let mut output = String::new();

        if self.highlighting_enabled {
            // Results header showing match count
            let match_word = if match_count == 1 { "match" } else { "matches" };
            output.push_str(&format!(
                "\x1b[32m✓\x1b[0m \x1b[90m{} {}\x1b[0m\n",
                match_count, match_word
            ));

            // Format based on output mode
            match output_mode {
                "files_with_matches" => {
                    // Just file paths - show them in dim color
                    for line in display_lines {
                        output.push_str(&format!("  \x1b[90m{}\x1b[0m\n", line));
                    }
                }
                "content" => {
                    // Content with line numbers - highlight the content portion
                    for line in display_lines {
                        let highlighted_line = highlight_grep_match(line);
                        output.push_str(&format!("  {}\n", highlighted_line));
                    }
                }
                "count" => {
                    // Just counts - show path:count pairs
                    for line in display_lines {
                        output.push_str(&format!("  \x1b[90m{}\x1b[0m\n", line));
                    }
                }
                _ => {
                    // Unknown mode - show raw
                    for line in display_lines {
                        output.push_str(&format!("  \x1b[90m{}\x1b[0m\n", line));
                    }
                }
            }

            if truncated {
                output.push_str(&format!(
                    "\x1b[90m... {} more lines\x1b[0m\n",
                    line_count - MAX_RESULT_LINES
                ));
            }
        } else {
            // Plain text format
            let match_word = if match_count == 1 { "match" } else { "matches" };
            output.push_str(&format!("{} {}\n", match_count, match_word));

            for line in display_lines {
                output.push_str(&format!("  {}\n", line));
            }

            if truncated {
                output.push_str(&format!(
                    "... {} more lines\n",
                    line_count - MAX_RESULT_LINES
                ));
            }
        }

        output
    }

    /// Render Glob result (verbose mode).
    ///
    /// Groups files by directory for readability, matching stream processor format.
    fn render_glob_result(&self, file_count: usize, content: &str, truncated: bool) -> String {
        const MAX_RESULT_LINES: usize = 200;

        // Empty result
        if content.is_empty() {
            return if self.highlighting_enabled {
                "\x1b[90m(no matches)\x1b[0m\n".to_string()
            } else {
                "(no matches)\n".to_string()
            };
        }

        // Parse file paths from content
        let files: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();

        // Group files by directory for readability
        let grouped = group_files_by_directory(&files);

        // Determine if we need to truncate based on total display lines
        let total_display_lines: usize = grouped
            .values()
            .map(|paths| paths.len() + 1) // +1 for directory header
            .sum();
        let should_truncate = truncated || total_display_lines > MAX_RESULT_LINES;

        let mut output = String::new();

        if self.highlighting_enabled {
            // Results header showing match count
            let file_word = if file_count == 1 { "file" } else { "files" };
            output.push_str(&format!(
                "\x1b[32m✓\x1b[0m \x1b[90m{} {} matched\x1b[0m\n",
                file_count, file_word
            ));

            // Display files grouped by directory
            let mut lines_shown = 0;
            for (dir, paths) in &grouped {
                if should_truncate && lines_shown >= MAX_RESULT_LINES {
                    break;
                }

                // Directory header
                if dir.is_empty() {
                    output.push_str("  \x1b[1m.\x1b[0m\n");
                } else {
                    output.push_str(&format!("  \x1b[1m{}/\x1b[0m\n", dir));
                }
                lines_shown += 1;

                // Files in this directory
                for path in paths {
                    if should_truncate && lines_shown >= MAX_RESULT_LINES {
                        break;
                    }
                    // Extract just the filename part
                    let filename = path.rsplit('/').next().unwrap_or(path);
                    output.push_str(&format!("    \x1b[90m{}\x1b[0m\n", filename));
                    lines_shown += 1;
                }
            }

            if should_truncate && lines_shown < file_count {
                output.push_str(&format!(
                    "\x1b[90m... {} more files\x1b[0m\n",
                    file_count.saturating_sub(lines_shown)
                ));
            }
        } else {
            // Plain text format
            let file_word = if file_count == 1 { "file" } else { "files" };
            output.push_str(&format!("{} {} matched\n", file_count, file_word));

            let mut lines_shown = 0;
            for (dir, paths) in &grouped {
                if should_truncate && lines_shown >= MAX_RESULT_LINES {
                    break;
                }

                // Directory header
                if dir.is_empty() {
                    output.push_str("  .\n");
                } else {
                    output.push_str(&format!("  {}/\n", dir));
                }
                lines_shown += 1;

                // Files in this directory
                for path in paths {
                    if should_truncate && lines_shown >= MAX_RESULT_LINES {
                        break;
                    }
                    let filename = path.rsplit('/').next().unwrap_or(path);
                    output.push_str(&format!("    {}\n", filename));
                    lines_shown += 1;
                }
            }

            if should_truncate && lines_shown < file_count {
                output.push_str(&format!(
                    "... {} more files\n",
                    file_count.saturating_sub(lines_shown)
                ));
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

/// Group file paths by their parent directory.
///
/// Returns a sorted map of directory -> list of full file paths.
fn group_files_by_directory<'a>(files: &[&'a str]) -> BTreeMap<String, Vec<&'a str>> {
    let mut grouped: BTreeMap<String, Vec<&'a str>> = BTreeMap::new();

    for file in files {
        let dir = match file.rfind('/') {
            Some(pos) => file[..pos].to_string(),
            None => String::new(), // No directory, use empty string for root
        };
        grouped.entry(dir).or_default().push(file);
    }

    grouped
}

/// Extract line number from a cat-n formatted line, if present.
/// Returns (line_number_str, rest_of_line) or None if not a cat-n line.
fn extract_line_number(line: &str) -> Option<(&str, &str)> {
    // Try to find separator: tab or arrow character
    let separator_pos = line.find('\t').or_else(|| line.find('→'));

    if let Some(sep_pos) = separator_pos {
        let prefix = &line[..sep_pos];
        // Arrow is multi-byte UTF-8, so we need to handle it properly
        let rest = if line[sep_pos..].starts_with('→') {
            &line[sep_pos + '→'.len_utf8()..]
        } else {
            &line[sep_pos + 1..] // tab is single byte
        };

        // Check if prefix is whitespace followed by digits
        let trimmed = prefix.trim_start();
        if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
            return Some((trimmed, rest));
        }
    }
    None
}

/// Highlight a grep match within a line of output.
///
/// Attempts to find and highlight the matched portion of the line.
/// For content mode output (filename:line_number:content), this highlights
/// the content portion where the pattern matched.
fn highlight_grep_match(line: &str) -> String {
    // Parse the line format: filename:line_number:content or just filename
    // We apply dim styling to the filename:line_number prefix
    // and yellow styling to the content

    // Try to find the pattern ":number:" which indicates content mode
    if let Some(first_colon) = line.find(':') {
        if let Some(second_colon_offset) = line[first_colon + 1..].find(':') {
            let second_colon = first_colon + 1 + second_colon_offset;
            // Check if the part between colons is a number
            let potential_line_num = &line[first_colon + 1..second_colon];
            if potential_line_num.chars().all(|c| c.is_ascii_digit()) {
                // This looks like filename:line_number:content format
                let prefix = &line[..second_colon + 1];
                let content = &line[second_colon + 1..];
                return format!("\x1b[90m{}\x1b[0m\x1b[93m{}\x1b[0m", prefix, content);
            }
        }
    }

    // Default: just show the line in dim color
    format!("\x1b[90m{}\x1b[0m", line)
}

/// Normalize Claude CLI's `cat -n` line number format to a cleaner pipe-separated format.
///
/// Transforms lines from:
/// - `     1\tcontent` → ` 1 │ content`
/// - `    12→content` → `12 │ content`
/// - `   123\tcontent` → `123 │ content`
///
/// Handles both tab (`\t`) and arrow (`→`) separators as Claude CLI may use either.
/// Line numbers are right-aligned to the width of the largest line number.
/// Lines that don't match the pattern are passed through unchanged.
fn normalize_cat_n_format(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();

    // First pass: find max line number width
    let max_width = lines
        .iter()
        .filter_map(|line| extract_line_number(line))
        .map(|(num, _)| num.len())
        .max()
        .unwrap_or(1);

    // Second pass: format with consistent width
    lines
        .iter()
        .map(|line| {
            if let Some((num, rest)) = extract_line_number(line) {
                format!("{:>width$} │ {}", num, rest, width = max_width)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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

    #[test]
    fn test_group_files_by_directory() {
        let files = vec!["src/main.rs", "src/lib.rs", "tests/test.rs", "Cargo.toml"];
        let grouped = group_files_by_directory(&files);
        assert_eq!(grouped.len(), 3);
        assert!(grouped.contains_key("src"));
        assert!(grouped.contains_key("tests"));
        assert!(grouped.contains_key("")); // root files
        assert_eq!(grouped.get("src").map(|v| v.len()), Some(2));
    }

    #[test]
    fn test_normalize_cat_n_format_single_digit() {
        let input = "     1\tfn main() {";
        let expected = "1 │ fn main() {";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_arrow_separator() {
        let input = "       1→fn main() {";
        let expected = "1 │ fn main() {";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_alignment() {
        let input = "       9→line nine\n      10→line ten";
        let expected = " 9 │ line nine\n10 │ line ten";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_highlight_grep_match_content_format() {
        let line = "src/main.rs:10:fn main() {}";
        let result = highlight_grep_match(line);
        // Should contain ANSI codes for highlighting
        assert!(result.contains("\x1b[90m"));
        assert!(result.contains("\x1b[93m"));
        assert!(result.contains("src/main.rs:10:"));
        assert!(result.contains("fn main() {}"));
    }

    #[test]
    fn test_highlight_grep_match_simple_path() {
        let line = "src/main.rs";
        let result = highlight_grep_match(line);
        // Should be dimmed but not split
        assert!(result.contains("\x1b[90m"));
        assert!(result.contains("src/main.rs"));
    }
}
