//! Shared tool rendering functions (Functional Core).
//!
//! This module contains pure rendering functions used by both stream processor
//! (live execution) and replay renderer. Functions accept data parameters directly
//! and return formatted strings, avoiding any I/O operations.
//!
//! # Design Principles
//!
//! 1. **Pure Functions**: All functions are stateless and free of side effects
//! 2. **Data-Centric**: Accept structured data, not processor/renderer references
//! 3. **Dual Mode**: Support both terminal (ANSI) and plain text output
//! 4. **Single Source of Truth**: One implementation for both rendering paths
//!
//! # Usage
//!
//! ```ignore
//! use ralph::render::{render_bash_invocation, RenderContext};
//!
//! let ctx = RenderContext::terminal(&highlighter);
//! let output = render_bash_invocation(&ctx, "ls -la", false);
//! ```

use super::utils::{
    extract_language_from_path, group_files_by_directory, highlight_grep_match,
    normalize_cat_n_format,
};
use crate::diff_highlight::highlight_with_basic_colors;
use crate::highlight::Highlighter;

// =============================================================================
// ANSI Color Constants
// =============================================================================

/// ANSI codes for consistent terminal styling across all renderers.
pub mod ansi {
    /// Cyan text (tool headers)
    pub const CYAN: &str = "\x1b[36m";
    /// Green text (success indicators)
    pub const GREEN: &str = "\x1b[32m";
    /// Red text (error indicators)
    pub const RED: &str = "\x1b[31m";
    /// Yellow text (warnings, highlights)
    pub const YELLOW: &str = "\x1b[33m";
    /// Dim text (secondary info)
    pub const DIM: &str = "\x1b[90m";
    /// Bold text
    pub const BOLD: &str = "\x1b[1m";
    /// Reset all formatting
    pub const RESET: &str = "\x1b[0m";
    /// Vibrant red background for "before" blocks in diffs (RGB: 140, 45, 45)
    /// Saturated enough to be immediately distinguishable, but not harsh
    pub const RED_BG: &str = "\x1b[48;2;140;45;45m";
    /// Vibrant green background for "after" blocks in diffs (RGB: 45, 130, 45)
    /// Saturated enough to be immediately distinguishable, but not harsh
    pub const GREEN_BG: &str = "\x1b[48;2;45;130;45m";
}

// =============================================================================
// Render Context
// =============================================================================

/// Configuration for rendering operations.
///
/// Holds shared state needed for rendering, including the code highlighter
/// and whether terminal features (ANSI codes) are enabled.
pub struct RenderContext<'a> {
    /// Code highlighter for syntax highlighting
    pub highlighter: &'a Highlighter,
    /// Whether ANSI color codes should be included
    pub terminal: bool,
}

impl<'a> RenderContext<'a> {
    /// Create a context for terminal rendering (with ANSI codes).
    pub fn terminal(highlighter: &'a Highlighter) -> Self {
        Self {
            highlighter,
            terminal: true,
        }
    }

    /// Create a context for plain text rendering (no ANSI codes).
    pub fn plain(highlighter: &'a Highlighter) -> Self {
        Self {
            highlighter,
            terminal: false,
        }
    }
}

// =============================================================================
// Tool Invocation Renderers
// =============================================================================

/// Render a Bash tool invocation.
///
/// Single-line commands show inline; multi-line commands are wrapped in code blocks.
pub fn render_bash_invocation(ctx: &RenderContext, command: &str) -> String {
    let is_multiline = command.contains('\n');

    if ctx.terminal {
        let mut output = String::new();
        output.push_str(&format!("{}▶ Bash{}\n", ansi::CYAN, ansi::RESET));

        if is_multiline {
            output.push_str("```sh\n");
            let highlighted = ctx.highlighter.highlight(command, Some("sh"));
            output.push_str(&highlighted);
            if !highlighted.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("```\n");
        } else {
            output.push_str("  ");
            let highlighted = ctx.highlighter.highlight(command, Some("sh"));
            let trimmed = highlighted.trim_end_matches(ansi::RESET);
            output.push_str(trimmed);
            output.push_str(&format!("{}\n", ansi::RESET));
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

/// Configuration for Grep tool invocation display.
pub struct GrepInvocationParams<'a> {
    pub pattern: &'a str,
    pub path: Option<&'a str>,
    pub output_mode: Option<&'a str>,
    pub glob: Option<&'a str>,
    pub file_type: Option<&'a str>,
    pub case_insensitive: bool,
}

/// Render a Grep tool invocation (verbose mode).
pub fn render_grep_invocation(ctx: &RenderContext, params: &GrepInvocationParams) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!("{}▶ Grep{}\n", ansi::CYAN, ansi::RESET));

        // Pattern with regex highlighting
        output.push_str(&format!("  {}Pattern:{} ", ansi::BOLD, ansi::RESET));
        let highlighted = ctx.highlighter.highlight(params.pattern, Some("regex"));
        output.push_str(&highlighted);
        output.push('\n');

        // Optional fields
        if let Some(p) = params.path {
            output.push_str(&format!("  {}Path:{} {}\n", ansi::DIM, ansi::RESET, p));
        }
        if let Some(mode) = params.output_mode {
            output.push_str(&format!("  {}Mode:{} {}\n", ansi::DIM, ansi::RESET, mode));
        }
        if let Some(g) = params.glob {
            output.push_str(&format!("  {}glob:{} {}\n", ansi::DIM, ansi::RESET, g));
        }
        if let Some(ft) = params.file_type {
            output.push_str(&format!("  {}type:{} {}\n", ansi::DIM, ansi::RESET, ft));
        }
        if params.case_insensitive {
            output.push_str(&format!(
                "  {}case-insensitive:{} true\n",
                ansi::DIM,
                ansi::RESET
            ));
        }
    } else {
        output.push_str("> Grep\n");
        output.push_str(&format!("  Pattern: {}\n", params.pattern));
        if let Some(p) = params.path {
            output.push_str(&format!("  Path: {}\n", p));
        }
        if let Some(mode) = params.output_mode {
            output.push_str(&format!("  Mode: {}\n", mode));
        }
        if let Some(g) = params.glob {
            output.push_str(&format!("  glob: {}\n", g));
        }
        if let Some(ft) = params.file_type {
            output.push_str(&format!("  type: {}\n", ft));
        }
        if params.case_insensitive {
            output.push_str("  case-insensitive: true\n");
        }
    }

    output
}

/// Render a Read tool invocation (verbose mode).
pub fn render_read_invocation(
    ctx: &RenderContext,
    file_path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!("{}▶ Read{}\n", ansi::CYAN, ansi::RESET));
        output.push_str(&format!("  {}{}{}\n", ansi::DIM, file_path, ansi::RESET));

        if let Some(off) = offset {
            output.push_str(&format!("  offset: {}{}{}\n", ansi::DIM, off, ansi::RESET));
        }
        if let Some(lim) = limit {
            output.push_str(&format!("  limit: {}{}{}\n", ansi::DIM, lim, ansi::RESET));
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

/// Render a Glob tool invocation (verbose mode).
pub fn render_glob_invocation(ctx: &RenderContext, pattern: &str, path: Option<&str>) -> String {
    // Default to current directory if path not provided
    let search_path = path.unwrap_or(".");

    if ctx.terminal {
        let mut output = String::new();
        output.push_str(&format!("{}▶ Glob{}\n", ansi::CYAN, ansi::RESET));
        output.push_str(&format!(
            "  {}Pattern:{} {}\n",
            ansi::BOLD,
            ansi::RESET,
            pattern
        ));
        output.push_str(&format!(
            "  {}Path:{} {}\n",
            ansi::DIM,
            ansi::RESET,
            search_path
        ));
        output
    } else {
        let mut output = String::new();
        output.push_str("> Glob\n");
        output.push_str(&format!("  Pattern: {}\n", pattern));
        output.push_str(&format!("  Path: {}\n", search_path));
        output
    }
}

/// Item for TodoWrite display.
pub struct TodoDisplayItem<'a> {
    pub content: &'a str,
    pub status: &'a str,
    pub active_form: Option<&'a str>,
}

/// Render a TodoWrite tool invocation (verbose mode).
pub fn render_todowrite_invocation(ctx: &RenderContext, todos: &[TodoDisplayItem]) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!("{}▶ TodoWrite{}\n", ansi::CYAN, ansi::RESET));

        if todos.is_empty() {
            output.push_str(&format!(
                "  {}(clearing todo list){}\n",
                ansi::DIM,
                ansi::RESET
            ));
        } else {
            for todo in todos {
                let status_icon = match todo.status {
                    "completed" => format!("{}✓{}", ansi::GREEN, ansi::RESET),
                    "in_progress" => format!("{}⋯{}", ansi::YELLOW, ansi::RESET),
                    _ => format!("{}○{}", ansi::DIM, ansi::RESET),
                };
                output.push_str(&format!("  {} {}", status_icon, todo.content));

                // Show activeForm if different from content
                if let Some(af) = todo.active_form {
                    if af != todo.content {
                        output.push_str(&format!(" {}({}){}", ansi::DIM, af, ansi::RESET));
                    }
                }
                output.push('\n');
            }
        }
    } else {
        output.push_str("> TodoWrite\n");

        if todos.is_empty() {
            output.push_str("  (clearing todo list)\n");
        } else {
            for todo in todos {
                let status_marker = match todo.status {
                    "completed" => "[x]",
                    "in_progress" => "[~]",
                    _ => "[ ]",
                };
                output.push_str(&format!("  {} {}", status_marker, todo.content));

                // Show activeForm if different from content
                if let Some(af) = todo.active_form {
                    if af != todo.content {
                        output.push_str(&format!(" ({})", af));
                    }
                }
                output.push('\n');
            }
        }
    }

    output
}

/// Render a default tool invocation (fallback for unknown tools).
pub fn render_default_invocation(
    ctx: &RenderContext,
    tool_name: &str,
    key_argument: Option<&str>,
) -> String {
    if ctx.terminal {
        format!(
            "{}▶ {}{}{}\n",
            ansi::CYAN,
            tool_name,
            ansi::RESET,
            if let Some(arg) = key_argument {
                format!(" {}{}{}", ansi::DIM, arg, ansi::RESET)
            } else {
                String::new()
            }
        )
    } else {
        format!("> {} {}\n", tool_name, key_argument.unwrap_or_default())
    }
}

// =============================================================================
// Tool Result Renderers
// =============================================================================

/// Render a Bash tool result.
pub fn render_bash_result(
    ctx: &RenderContext,
    is_error: bool,
    content: Option<&str>,
    truncated: bool,
) -> String {
    // Treat empty string the same as None for display purposes
    let content = content.filter(|c| !c.is_empty());

    if ctx.terminal {
        if is_error {
            let mut output = format!("{}✗ Error{}\n", ansi::RED, ansi::RESET);
            if let Some(c) = content {
                output.push_str(&format!("{}{}{}\n", ansi::DIM, c, ansi::RESET));
            }
            output
        } else if let Some(c) = content {
            let mut output = format!("{}{}{}\n", ansi::DIM, c, ansi::RESET);
            if truncated {
                output.push_str(&format!("{}(output truncated){}\n", ansi::DIM, ansi::RESET));
            }
            output
        } else {
            format!("{}✓ (ok){}\n", ansi::GREEN, ansi::RESET)
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
        "(ok)\n".to_string()
    }
}

/// Render content with colored line numbers (for before/after blocks).
pub fn render_content_block(
    ctx: &RenderContext,
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

        if ctx.terminal {
            // Line number with background color
            let bg_color = if is_before {
                ansi::RED_BG
            } else {
                ansi::GREEN_BG
            };
            output.push_str(&format!("{}{} {} ", bg_color, line_num_str, ansi::RESET));

            // Content with syntax highlighting
            let highlighted = if let Some(lang) = language {
                ctx.highlighter.highlight(line, Some(lang))
            } else {
                line.to_string()
            };
            output.push_str(highlighted.trim_end_matches(ansi::RESET));
            output.push_str(&format!("{}\n", ansi::RESET));
        } else {
            output.push_str(&format!("{} │ {}\n", line_num_str, line));
        }
    }

    output
}

/// Render an Edit result with before/after blocks.
pub fn render_edit_before_after(
    ctx: &RenderContext,
    file_path: &str,
    old_content: &str,
    new_content: &str,
) -> String {
    let mut output = String::new();

    // File header
    if ctx.terminal {
        output.push_str(&format!("{}{}{}\n", ansi::DIM, file_path, ansi::RESET));
    } else {
        output.push_str(&format!("{}\n", file_path));
    }

    // Detect language for syntax highlighting
    let language = extract_language_from_path(file_path);

    // Before block
    output.push_str(&render_content_block(ctx, old_content, language, true));

    // Separator
    if ctx.terminal {
        output.push_str(&format!(
            "{}────────────────────{}\n",
            ansi::DIM,
            ansi::RESET
        ));
    } else {
        output.push_str("────────────────────\n");
    }

    // After block
    output.push_str(&render_content_block(ctx, new_content, language, false));

    output
}

/// Render an Edit diff result.
pub fn render_edit_diff(ctx: &RenderContext, file_path: &str, diff_content: &str) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!("{}{}{}\n", ansi::DIM, file_path, ansi::RESET));
        output.push_str(&highlight_with_basic_colors(diff_content));
    } else {
        output.push_str(&format!("{}\n", file_path));
        output.push_str(diff_content);
    }
    output.push('\n');

    output
}

/// Render a "no changes" message for Edit/Write tools.
pub fn render_no_changes_message(ctx: &RenderContext, file_path: &str, tool: &str) -> String {
    if ctx.terminal {
        format!(
            "{}{}{}\n{}⚠ No changes ({}){}\n",
            ansi::DIM,
            file_path,
            ansi::RESET,
            ansi::YELLOW,
            tool,
            ansi::RESET
        )
    } else {
        format!("{}\nNo changes ({})\n", file_path, tool)
    }
}

/// Render a Write new file result.
pub fn render_write_new_file(ctx: &RenderContext, file_path: &str, content: &str) -> String {
    let mut output = String::new();

    // File header with (new file) indicator
    if ctx.terminal {
        output.push_str(&format!(
            "{}{}{} {}(new file){}\n",
            ansi::DIM,
            file_path,
            ansi::RESET,
            ansi::GREEN,
            ansi::RESET
        ));
    } else {
        output.push_str(&format!("{} (new file)\n", file_path));
    }

    // Content with green background line numbers
    let language = extract_language_from_path(file_path);
    output.push_str(&render_content_block(ctx, content, language, false));

    output
}

/// Render a Write no changes result.
pub fn render_write_no_changes(ctx: &RenderContext, file_path: &str, is_new_file: bool) -> String {
    if is_new_file {
        if ctx.terminal {
            format!(
                "{}{}{} {}(new file){}\n{}⚠ Empty file created{}\n",
                ansi::DIM,
                file_path,
                ansi::RESET,
                ansi::GREEN,
                ansi::RESET,
                ansi::YELLOW,
                ansi::RESET
            )
        } else {
            format!("{} (new file)\nEmpty file created\n", file_path)
        }
    } else {
        render_no_changes_message(ctx, file_path, "write")
    }
}

/// Render a Read tool result (verbose mode).
pub fn render_read_result(
    ctx: &RenderContext,
    file_path: &str,
    content: &str,
    line_count: usize,
    truncated: bool,
) -> String {
    const MAX_CONTENT_LINES: usize = 100;

    // Empty result
    if content.is_empty() {
        return if ctx.terminal {
            format!("{}(empty file){}\n", ansi::DIM, ansi::RESET)
        } else {
            "(empty file)\n".to_string()
        };
    }

    // Check for binary file indicator
    if content.contains("(binary file)") || content.starts_with('\u{0}') {
        return if ctx.terminal {
            format!("{}(binary file){}\n", ansi::DIM, ansi::RESET)
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

    if ctx.terminal {
        // Results header showing line count
        let line_word = if line_count == 1 { "line" } else { "lines" };
        output.push_str(&format!(
            "{}✓{} {}{} {}{}\n",
            ansi::GREEN,
            ansi::RESET,
            ansi::DIM,
            line_count,
            line_word,
            ansi::RESET
        ));

        // Apply syntax highlighting to the content
        let content_to_highlight = display_lines.join("\n");
        let highlighted = if language.is_some() {
            ctx.highlighter.highlight(&content_to_highlight, language)
        } else {
            content_to_highlight.clone()
        };

        // Display highlighted content with indentation
        for line in highlighted.lines() {
            output.push_str(&format!("  {}\n", line));
        }

        if should_truncate {
            output.push_str(&format!(
                "{}... {} more lines{}\n",
                ansi::DIM,
                actual_line_count.saturating_sub(MAX_CONTENT_LINES),
                ansi::RESET
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

/// Render a Grep tool result (verbose mode).
pub fn render_grep_result(
    ctx: &RenderContext,
    match_count: usize,
    output_mode: &str,
    content: &str,
) -> String {
    const MAX_RESULT_LINES: usize = 100;

    // Empty result
    if content.is_empty() {
        return if ctx.terminal {
            format!("{}(no matches){}\n", ansi::DIM, ansi::RESET)
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

    if ctx.terminal {
        // Results header showing match count
        let match_word = if match_count == 1 { "match" } else { "matches" };
        output.push_str(&format!(
            "{}✓{} {}{} {}{}\n",
            ansi::GREEN,
            ansi::RESET,
            ansi::DIM,
            match_count,
            match_word,
            ansi::RESET
        ));

        // Format based on output mode
        match output_mode {
            "files_with_matches" => {
                for line in display_lines {
                    output.push_str(&format!("  {}{}{}\n", ansi::DIM, line, ansi::RESET));
                }
            }
            "content" => {
                for line in display_lines {
                    let highlighted_line = highlight_grep_match(line);
                    output.push_str(&format!("  {}\n", highlighted_line));
                }
            }
            _ => {
                // count mode and other modes
                for line in display_lines {
                    output.push_str(&format!("  {}{}{}\n", ansi::DIM, line, ansi::RESET));
                }
            }
        }

        if truncated {
            output.push_str(&format!(
                "{}... {} more lines{}\n",
                ansi::DIM,
                line_count - MAX_RESULT_LINES,
                ansi::RESET
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

/// Render a Glob tool result (verbose mode).
pub fn render_glob_result(
    ctx: &RenderContext,
    file_count: usize,
    content: &str,
    truncated: bool,
) -> String {
    const MAX_RESULT_LINES: usize = 200;

    // Empty result
    if content.is_empty() {
        return if ctx.terminal {
            format!("{}(no matches){}\n", ansi::DIM, ansi::RESET)
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

    if ctx.terminal {
        // Results header showing match count
        let file_word = if file_count == 1 { "file" } else { "files" };
        output.push_str(&format!(
            "{}✓{} {}{} {} matched{}\n",
            ansi::GREEN,
            ansi::RESET,
            ansi::DIM,
            file_count,
            file_word,
            ansi::RESET
        ));

        // Display files grouped by directory
        let mut lines_shown = 0;
        for (dir, paths) in &grouped {
            if should_truncate && lines_shown >= MAX_RESULT_LINES {
                break;
            }

            // Directory header
            if dir.is_empty() {
                output.push_str(&format!("  {}.{}\n", ansi::BOLD, ansi::RESET));
            } else {
                output.push_str(&format!("  {}{}/{}\n", ansi::BOLD, dir, ansi::RESET));
            }
            lines_shown += 1;

            // Files in this directory
            for path in paths {
                if should_truncate && lines_shown >= MAX_RESULT_LINES {
                    break;
                }
                let filename = path.rsplit('/').next().unwrap_or(path);
                output.push_str(&format!("    {}{}{}\n", ansi::DIM, filename, ansi::RESET));
                lines_shown += 1;
            }
        }

        if should_truncate && lines_shown < file_count {
            output.push_str(&format!(
                "{}... {} more files{}\n",
                ansi::DIM,
                file_count.saturating_sub(lines_shown),
                ansi::RESET
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

/// Render a TodoWrite result.
pub fn render_todowrite_result(
    ctx: &RenderContext,
    is_error: bool,
    message: Option<&str>,
) -> String {
    if ctx.terminal {
        if is_error {
            format!(
                "{}✗{} {}\n",
                ansi::RED,
                ansi::RESET,
                message.unwrap_or("Failed to update todos")
            )
        } else {
            format!(
                "{}✓{} {}\n",
                ansi::GREEN,
                ansi::RESET,
                message.unwrap_or("Todos updated")
            )
        }
    } else if is_error {
        format!("! {}\n", message.unwrap_or("Failed to update todos"))
    } else {
        format!("  {}\n", message.unwrap_or("Todos updated"))
    }
}

/// Parameters for rendering a NotebookEdit result.
pub struct NotebookEditParams<'a> {
    pub notebook_path: &'a str,
    pub cell_identifier: &'a str,
    pub cell_type: Option<&'a str>,
    pub edit_mode: &'a str,
    pub diff_content: &'a str,
}

/// Render a NotebookEdit result.
pub fn render_notebook_edit(ctx: &RenderContext, params: &NotebookEditParams) -> String {
    let mut output = String::new();

    if ctx.terminal {
        output.push_str(&format!(
            "{}{}{} cell {} ({}) [{}]\n",
            ansi::DIM,
            params.notebook_path,
            ansi::RESET,
            params.cell_identifier,
            params.cell_type.unwrap_or("code"),
            params.edit_mode
        ));
        output.push_str(&highlight_with_basic_colors(params.diff_content));
    } else {
        output.push_str(&format!(
            "{} cell {} ({}) [{}]\n",
            params.notebook_path,
            params.cell_identifier,
            params.cell_type.unwrap_or("code"),
            params.edit_mode
        ));
        output.push_str(params.diff_content);
    }
    output.push('\n');

    output
}

/// Render a default tool result (fallback for unknown tools).
pub fn render_default_result(ctx: &RenderContext, is_error: bool, content: Option<&str>) -> String {
    let display_content = content.unwrap_or("(no output)");

    if ctx.terminal {
        if is_error {
            format!("{}✗ Error:{} {}\n", ansi::RED, ansi::RESET, display_content)
        } else {
            format!(
                "{}✓{} {}{}{}\n",
                ansi::GREEN,
                ansi::RESET,
                ansi::DIM,
                display_content,
                ansi::RESET
            )
        }
    } else if is_error {
        format!("! Error: {}\n", display_content)
    } else {
        format!("  {}\n", display_content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::ThemeConfig;

    fn test_highlighter() -> Highlighter {
        Highlighter::with_config(ThemeConfig::default()).unwrap()
    }

    // =========================================================================
    // Bash Invocation Tests
    // =========================================================================

    #[test]
    fn test_render_bash_invocation_single_line_terminal() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::terminal(&highlighter);
        let output = render_bash_invocation(&ctx, "ls -la");

        assert!(
            output.contains("▶ Bash"),
            "Missing header. Output: {:?}",
            output
        );
        // Command parts may be split by ANSI codes from syntax highlighting
        assert!(output.contains("ls"), "Missing 'ls'. Output: {:?}", output);
        assert!(output.contains("la"), "Missing 'la'. Output: {:?}", output);
        assert!(output.contains("\x1b[")); // ANSI codes
    }

    #[test]
    fn test_render_bash_invocation_single_line_plain() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let output = render_bash_invocation(&ctx, "ls -la");

        assert!(output.contains("> Bash"));
        assert!(output.contains("ls -la"));
        assert!(!output.contains("\x1b[")); // No ANSI codes
    }

    #[test]
    fn test_render_bash_invocation_multiline() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let output = render_bash_invocation(&ctx, "echo 'line1'\necho 'line2'");

        assert!(output.contains("> Bash"));
        assert!(output.contains("```sh"));
        assert!(output.contains("echo 'line1'"));
    }

    // =========================================================================
    // Grep Invocation Tests
    // =========================================================================

    #[test]
    fn test_render_grep_invocation_minimal() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let params = GrepInvocationParams {
            pattern: "fn main",
            path: None,
            output_mode: None,
            glob: None,
            file_type: None,
            case_insensitive: false,
        };
        let output = render_grep_invocation(&ctx, &params);

        assert!(output.contains("> Grep"));
        assert!(output.contains("Pattern: fn main"));
    }

    #[test]
    fn test_render_grep_invocation_full() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let params = GrepInvocationParams {
            pattern: "TODO",
            path: Some("src/"),
            output_mode: Some("content"),
            glob: Some("*.rs"),
            file_type: Some("rust"),
            case_insensitive: true,
        };
        let output = render_grep_invocation(&ctx, &params);

        assert!(output.contains("Pattern: TODO"));
        assert!(output.contains("Path: src/"));
        assert!(output.contains("Mode: content"));
        assert!(output.contains("glob: *.rs"));
        assert!(output.contains("type: rust"));
        assert!(output.contains("case-insensitive: true"));
    }

    // =========================================================================
    // Read Result Tests
    // =========================================================================

    #[test]
    fn test_render_read_result_empty() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let output = render_read_result(&ctx, "test.rs", "", 0, false);

        assert!(output.contains("(empty file)"));
    }

    #[test]
    fn test_render_read_result_normalizes_cat_n() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let output = render_read_result(&ctx, "test.rs", "     1\tfn main() {}", 1, false);

        assert!(output.contains("1 │ fn main() {}"));
        assert!(output.contains("1 line"));
    }

    // =========================================================================
    // Glob Result Tests
    // =========================================================================

    #[test]
    fn test_render_glob_result_empty() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let output = render_glob_result(&ctx, 0, "", false);

        assert!(output.contains("(no matches)"));
    }

    #[test]
    fn test_render_glob_result_grouped() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let output = render_glob_result(&ctx, 3, "src/main.rs\nsrc/lib.rs\ntests/test.rs", false);

        assert!(output.contains("3 files matched"));
        assert!(output.contains("src/"));
        assert!(output.contains("tests/"));
    }

    // =========================================================================
    // Edit Before/After Tests
    // =========================================================================

    #[test]
    fn test_render_edit_before_after_plain() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let output = render_edit_before_after(&ctx, "test.rs", "let x = 1;", "let x = 2;");

        assert!(output.contains("test.rs"));
        assert!(output.contains("let x = 1;"));
        assert!(output.contains("────────────────────"));
        assert!(output.contains("let x = 2;"));
    }

    // =========================================================================
    // TodoWrite Tests
    // =========================================================================

    #[test]
    fn test_render_todowrite_invocation() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let todos = vec![
            TodoDisplayItem {
                content: "Task 1",
                status: "completed",
                active_form: None,
            },
            TodoDisplayItem {
                content: "Task 2",
                status: "in_progress",
                active_form: Some("Working on Task 2"),
            },
            TodoDisplayItem {
                content: "Task 3",
                status: "pending",
                active_form: None,
            },
        ];
        let output = render_todowrite_invocation(&ctx, &todos);

        assert!(output.contains("> TodoWrite"));
        assert!(output.contains("[x] Task 1"));
        assert!(output.contains("[~] Task 2 (Working on Task 2)"));
        assert!(output.contains("[ ] Task 3"));
    }

    #[test]
    fn test_render_todowrite_result_success() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let output = render_todowrite_result(&ctx, false, Some("Todos updated"));

        assert!(output.contains("Todos updated"));
        assert!(!output.contains("!"));
    }

    #[test]
    fn test_render_todowrite_result_error() {
        let highlighter = test_highlighter();
        let ctx = RenderContext::plain(&highlighter);
        let output = render_todowrite_result(&ctx, true, Some("Failed"));

        assert!(output.contains("! Failed"));
    }
}
