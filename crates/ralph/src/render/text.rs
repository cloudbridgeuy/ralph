//! Shared text block rendering functions.
//!
//! This module provides unified rendering for text blocks (prose, code, diff)
//! used by both the stream processor (live execution) and replay renderer.
//!
//! # Design
//!
//! Text rendering is consolidated here to ensure visual consistency between
//! live streaming and replay. Both paths produce identical output for the
//! same input data.

use crate::diff_highlight::highlight_with_basic_colors;
use ralph_core::chunk::{ChunkType, ParsedChunk};
use termimad::MadSkin;

use super::RenderContext;

/// Render a text block (prose, code, or diff) to a string.
///
/// This is the canonical text rendering function used by both stream processor
/// and replay renderer. It handles:
/// - Prose: Optional markdown formatting via termimad
/// - Code: Fenced code blocks with syntax highlighting
/// - Diff: Fenced diff blocks with +/- coloring
///
/// # Arguments
///
/// * `ctx` - Render context with highlighter and terminal flag
/// * `chunk` - The parsed chunk to render
/// * `markdown_skin` - Optional markdown skin for prose rendering
///
/// # Returns
///
/// The rendered text as a string.
pub fn render_text_block(
    ctx: &RenderContext<'_>,
    chunk: &ParsedChunk,
    markdown_skin: Option<&MadSkin>,
) -> String {
    match &chunk.chunk_type {
        ChunkType::Prose => {
            if ctx.terminal {
                markdown_skin
                    .map(|skin| {
                        let rendered = skin.term_text(&chunk.content).to_string();
                        enhance_tables(&rendered)
                    })
                    .unwrap_or_else(|| chunk.content.clone())
            } else {
                chunk.content.clone()
            }
        }
        ChunkType::Code { language } => render_code_block(ctx, &chunk.content, language.as_deref()),
        ChunkType::Diff => render_diff_block(ctx, &chunk.content),
        ChunkType::Directive { verb, target } => {
            render_directive_block(ctx, &chunk.content, verb, target)
        }
    }
}

// --- Table enhancement (post-processing) ---

fn is_table_line(line: &str) -> bool {
    line.contains('│') || line.contains('├')
}

fn is_separator_line(line: &str) -> bool {
    line.contains('├')
}

fn pad_table_content_line(line: &str) -> String {
    let parts: Vec<&str> = line.split('│').collect();
    if parts.len() < 3 {
        return line.to_string();
    }
    let leading = parts[0];
    let interior = &parts[1..parts.len() - 1];
    let trailing = parts[parts.len() - 1];

    let padded_cells: Vec<String> = interior.iter().map(|p| format!(" {p} ")).collect();
    format!("{leading}│{}│{trailing}", padded_cells.join("│"))
}

fn widen_separator_line(line: &str) -> String {
    let mut result = String::new();
    let mut in_dash_run = false;
    for ch in line.chars() {
        match ch {
            '─' => {
                in_dash_run = true;
                result.push(ch);
            }
            '├' | '┼' | '┤' => {
                if in_dash_run {
                    result.push('─');
                    result.push('─');
                    in_dash_run = false;
                }
                result.push(ch);
            }
            _ => {
                result.push(ch);
            }
        }
    }
    result
}

fn map_border_chars(separator: &str, left: char, cross: char, right: char) -> String {
    separator
        .chars()
        .map(|ch| match ch {
            '├' => left,
            '┼' => cross,
            '┤' => right,
            _ => ch,
        })
        .collect()
}

fn derive_top_border(separator: &str) -> String {
    map_border_chars(separator, '┌', '┬', '┐')
}

fn derive_bottom_border(separator: &str) -> String {
    map_border_chars(separator, '└', '┴', '┘')
}

fn enhance_tables(rendered: &str) -> String {
    let lines: Vec<&str> = rendered.lines().collect();

    // Phase 1: Find table regions (start inclusive, end exclusive)
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if is_table_line(lines[i]) {
            let start = i;
            while i < lines.len() && is_table_line(lines[i]) {
                i += 1;
            }
            regions.push((start, i));
        } else {
            i += 1;
        }
    }

    // Phase 2: Process each region back-to-front (keeps indices stable)
    let mut result_lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();

    for &(start, end) in regions.iter().rev() {
        let separator_idx = (start..end).find(|&idx| is_separator_line(&result_lines[idx]));

        for line in result_lines.iter_mut().take(end).skip(start) {
            if is_separator_line(line) {
                *line = widen_separator_line(line);
            } else {
                *line = pad_table_content_line(line);
            }
        }

        if let Some(sep_idx) = separator_idx {
            let bottom = derive_bottom_border(&result_lines[sep_idx]);
            let top = derive_top_border(&result_lines[sep_idx]);
            result_lines.insert(end, bottom);
            result_lines.insert(start, top);
        }
    }

    let mut result = result_lines.join("\n");
    if rendered.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Render a fenced code block with syntax highlighting.
///
/// This is the shared implementation used by both stream processor and replay
/// renderer for code block rendering with visible fences.
///
/// # Arguments
///
/// * `ctx` - Render context with highlighter and terminal flag
/// * `content` - The code content to render
/// * `language` - Optional language hint for syntax highlighting
///
/// # Returns
///
/// The rendered code block with opening and closing fences.
pub fn render_code_block(ctx: &RenderContext<'_>, content: &str, language: Option<&str>) -> String {
    // Format the opening fence with language hint
    let lang_suffix = language.filter(|l| !l.is_empty()).unwrap_or("");
    let opening_fence = format!("```{lang_suffix}");

    // Highlight the code content (or leave plain if highlighting disabled)
    let highlighted_content = if ctx.terminal {
        ctx.highlighter.highlight(content, language)
    } else {
        content.to_string()
    };

    // Build the full block with fences
    format!("{}\n{}\n```", opening_fence, highlighted_content)
}

/// Render a fenced diff block with +/- coloring.
///
/// This is the shared implementation used by both stream processor and replay
/// renderer for diff block rendering with visible fences.
///
/// # Arguments
///
/// * `ctx` - Render context with highlighter and terminal flag
/// * `content` - The diff content to render
///
/// # Returns
///
/// The rendered diff block with opening and closing fences.
pub fn render_diff_block(ctx: &RenderContext<'_>, content: &str) -> String {
    let highlighted_content = if ctx.terminal {
        highlight_with_basic_colors(content)
    } else {
        content.to_string()
    };

    format!("```diff\n{}\n```", highlighted_content)
}

/// Render a directive block.
///
/// Returns an empty string — directive payloads are rendered as a banner
/// by the orchestrator display after streaming completes, not inline.
fn render_directive_block(
    _ctx: &RenderContext<'_>,
    _payload: &str,
    _verb: &str,
    _target: &str,
) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::{Highlighter, ThemeConfig};

    fn create_test_context() -> (Highlighter, MadSkin) {
        let highlighter = Highlighter::with_config(ThemeConfig::default()).unwrap();
        let skin = crate::markdown::create_markdown_skin();
        (highlighter, skin)
    }

    #[test]
    fn test_render_prose_plain() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::plain(&highlighter);
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Prose,
            content: "Hello, world!".to_string(),
        };

        let result = render_text_block(&ctx, &chunk, Some(&skin));
        assert_eq!(result, "Hello, world!");
    }

    #[test]
    fn test_render_prose_terminal_with_skin() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::terminal(&highlighter);
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Prose,
            content: "**bold** text".to_string(),
        };

        let result = render_text_block(&ctx, &chunk, Some(&skin));
        // Should contain the text and possibly ANSI codes
        assert!(result.contains("bold"));
        assert!(result.contains("text"));
    }

    #[test]
    fn test_render_prose_terminal_without_skin() {
        let (highlighter, _skin) = create_test_context();
        let ctx = RenderContext::terminal(&highlighter);
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Prose,
            content: "plain prose".to_string(),
        };

        let result = render_text_block(&ctx, &chunk, None);
        assert_eq!(result, "plain prose");
    }

    #[test]
    fn test_render_code_plain() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::plain(&highlighter);
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Code {
                language: Some("rust".to_string()),
            },
            content: "fn main() {}".to_string(),
        };

        let result = render_text_block(&ctx, &chunk, Some(&skin));
        assert!(result.starts_with("```rust\n"));
        assert!(result.contains("fn main() {}"));
        assert!(result.ends_with("\n```"));
    }

    #[test]
    fn test_render_code_no_language() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::plain(&highlighter);
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Code { language: None },
            content: "some code".to_string(),
        };

        let result = render_text_block(&ctx, &chunk, Some(&skin));
        assert!(result.starts_with("```\n"));
        assert!(result.contains("some code"));
    }

    #[test]
    fn test_render_code_terminal() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::terminal(&highlighter);
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Code {
                language: Some("rust".to_string()),
            },
            content: "fn main() {}".to_string(),
        };

        let result = render_text_block(&ctx, &chunk, Some(&skin));
        assert!(result.contains("```rust"));
        // Content is present (may be interspersed with ANSI codes)
        assert!(result.contains("fn"));
        assert!(result.contains("main"));
        // Terminal mode includes ANSI codes for syntax highlighting
        assert!(result.contains("\x1b["));
    }

    #[test]
    fn test_render_diff_plain() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::plain(&highlighter);
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Diff,
            content: "+added\n-removed".to_string(),
        };

        let result = render_text_block(&ctx, &chunk, Some(&skin));
        assert!(result.starts_with("```diff\n"));
        assert!(result.contains("+added"));
        assert!(result.contains("-removed"));
        assert!(result.ends_with("\n```"));
    }

    #[test]
    fn test_render_diff_terminal() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::terminal(&highlighter);
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Diff,
            content: "+added\n-removed".to_string(),
        };

        let result = render_text_block(&ctx, &chunk, Some(&skin));
        assert!(result.contains("```diff"));
        // Should contain ANSI codes for diff coloring
        assert!(result.contains("\x1b["));
    }

    // --- Table enhancement tests ---

    #[test]
    fn test_is_table_line() {
        assert!(is_table_line("│Feature│Status│"));
        assert!(is_table_line("├───────┼──────┤"));
        assert!(!is_table_line("just text"));
        assert!(!is_table_line(""));
    }

    #[test]
    fn test_is_separator_line() {
        assert!(is_separator_line("├───────┼──────┤"));
        assert!(!is_separator_line("│Feature│Status│"));
        assert!(!is_separator_line("just text"));
    }

    #[test]
    fn test_pad_table_content_line() {
        assert_eq!(
            pad_table_content_line("│Feature│Status│"),
            "│ Feature │ Status │"
        );
    }

    #[test]
    fn test_pad_table_content_line_preserves_leading() {
        assert_eq!(pad_table_content_line("  │A│B│"), "  │ A │ B │");
    }

    #[test]
    fn test_pad_not_a_table_line() {
        assert_eq!(pad_table_content_line("no pipes"), "no pipes");
        assert_eq!(pad_table_content_line("one│pipe"), "one│pipe");
    }

    #[test]
    fn test_widen_separator_line() {
        assert_eq!(
            widen_separator_line("├───────┼──────┤"),
            "├─────────┼────────┤"
        );
    }

    #[test]
    fn test_widen_separator_single_column() {
        assert_eq!(widen_separator_line("├───┤"), "├─────┤");
    }

    #[test]
    fn test_derive_top_border() {
        assert_eq!(
            derive_top_border("├─────────┼────────┤"),
            "┌─────────┬────────┐"
        );
    }

    #[test]
    fn test_derive_bottom_border() {
        assert_eq!(
            derive_bottom_border("├─────────┼────────┤"),
            "└─────────┴────────┘"
        );
    }

    #[test]
    fn test_enhance_tables_no_table() {
        let input = "Hello world\nNo tables here";
        assert_eq!(enhance_tables(input), input);
    }

    #[test]
    fn test_enhance_tables_full() {
        let input = "│Feature│Status│\n├───────┼──────┤\n│Auth   │Done  │";
        let result = enhance_tables(input);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "┌─────────┬────────┐");
        assert_eq!(lines[1], "│ Feature │ Status │");
        assert_eq!(lines[2], "├─────────┼────────┤");
        assert_eq!(lines[3], "│ Auth    │ Done   │");
        assert_eq!(lines[4], "└─────────┴────────┘");
    }

    #[test]
    fn test_enhance_tables_preserves_surrounding_text() {
        let input = "Before\n│A│B│\n├─┼─┤\n│C│D│\nAfter";
        let result = enhance_tables(input);
        assert!(result.starts_with("Before\n"));
        assert!(result.ends_with("\nAfter"));
        assert!(result.contains("┌"));
        assert!(result.contains("┘"));
    }

    #[test]
    fn test_enhance_tables_no_separator() {
        let input = "│A│B│\n│C│D│";
        let result = enhance_tables(input);
        // Without a separator, no borders are added, but cells are padded
        assert!(result.contains("│ A │ B │"));
        assert!(!result.contains("┌"));
    }

    #[test]
    fn test_enhance_tables_trailing_newline_preserved() {
        let input = "│A│B│\n├─┼─┤\n│C│D│\n";
        let result = enhance_tables(input);
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn test_enhance_tables_multiple_tables() {
        let input = "│A│B│\n├─┼─┤\n│C│D│\ntext\n│E│F│\n├─┼─┤\n│G│H│";
        let result = enhance_tables(input);
        // Both tables should have top and bottom borders
        let top_count = result.matches('┌').count();
        let bottom_count = result.matches('└').count();
        assert_eq!(top_count, 2);
        assert_eq!(bottom_count, 2);
    }

    // --- Directive rendering tests ---

    #[test]
    fn test_render_directive_returns_empty() {
        let (highlighter, _skin) = create_test_context();
        let ctx = RenderContext::terminal(&highlighter);
        let result = render_directive_block(&ctx, "Please review this", "ask", "reviewer");
        assert!(result.is_empty());
    }

    #[test]
    fn test_render_directive_plain_returns_empty() {
        let (highlighter, _skin) = create_test_context();
        let ctx = RenderContext::plain(&highlighter);
        let result = render_directive_block(&ctx, "Please review this", "ask", "reviewer");
        assert!(result.is_empty());
    }

    #[test]
    fn test_render_directive_via_render_text_block_returns_empty() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::plain(&highlighter);
        let chunk = ParsedChunk::directive("Review please", "ask", "reviewer");
        let result = render_text_block(&ctx, &chunk, Some(&skin));
        assert!(result.is_empty());
    }
}
