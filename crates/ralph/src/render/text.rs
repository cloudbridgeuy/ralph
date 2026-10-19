//! Shared text block rendering functions.
//!
//! This module provides unified rendering for text blocks (prose, code, diff)
//! used by the stream processor during live execution.

use crate::diff_highlight::highlight_with_basic_colors;
use ralph_core::chunk::{parse_chunks, ChunkType, ParsedChunk};
use termimad::terminal_size;

use super::link::{mark_links, paint_links};
use super::wrap::{render_prose, wrap_width};
use super::RenderContext;
use crate::markdown::MarkdownTheme;

pub fn render_text_block(
    ctx: &RenderContext<'_>,
    chunk: &ParsedChunk,
    markdown: Option<&MarkdownTheme>,
) -> String {
    match &chunk.chunk_type {
        ChunkType::Prose => {
            if ctx.terminal {
                markdown
                    .map(|theme| {
                        let width = wrap_width(terminal_size().0.into());
                        let rendered =
                            render_prose(&theme.skin, &mark_links(&chunk.content), width);
                        paint_links(&enhance_tables(&rendered), &theme.palette)
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

pub fn render_markdown(ctx: &RenderContext<'_>, text: &str, markdown: &MarkdownTheme) -> String {
    parse_chunks(text)
        .iter()
        .map(|chunk| {
            let rendered = render_text_block(ctx, chunk, Some(markdown));
            match chunk.chunk_type {
                ChunkType::Prose => rendered,
                _ => rendered + "\n",
            }
        })
        .collect()
}

// --- Table enhancement (post-processing) ---

fn is_table_line(line: &str) -> bool {
    line.contains('│') || line.contains('├')
}

fn is_separator_line(line: &str) -> bool {
    line.contains('├')
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
/// Renders code blocks with visible fences.
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

    if ctx.terminal && language == Some("mermaid") {
        if let Some(diagram) = (ctx.mermaid)(content, ctx.width) {
            return format!("{opening_fence}\n{diagram}\n```");
        }
    }

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
/// Renders diff blocks with visible fences.
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

    fn create_test_context() -> (Highlighter, MarkdownTheme) {
        let highlighter = Highlighter::with_config(ThemeConfig::default()).unwrap();
        let skin = MarkdownTheme::default();
        (highlighter, skin)
    }

    #[test]
    fn mermaid_block_uses_renderer_and_falls_back_to_highlight() {
        let (highlighter, _) = create_test_context();
        let mut ctx = RenderContext::terminal(&highlighter);
        ctx.mermaid = |_, _| Some("DIAGRAM".to_string());
        assert_eq!(
            render_code_block(&ctx, "graph TD\nA-->B", Some("mermaid")),
            "```mermaid\nDIAGRAM\n```"
        );
        ctx.mermaid = |_, _| None;
        let fallback = render_code_block(&ctx, "graph TD\nA-->B", Some("mermaid"));
        assert_eq!(
            fallback,
            format!(
                "```mermaid\n{}\n```",
                highlighter.highlight("graph TD\nA-->B", Some("mermaid"))
            )
        );
        let plain = RenderContext::plain(&highlighter);
        assert_eq!(
            render_code_block(&plain, "graph TD", Some("mermaid")),
            "```mermaid\ngraph TD\n```"
        );
    }

    #[test]
    fn render_markdown_plain_keeps_source_text() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::plain(&highlighter);
        let text = "[worker A · done]\n# Title\n\n```rust\nfn main() {}\n```\nend";
        let rendered = render_markdown(&ctx, text, &skin);
        assert!(!rendered.contains('\x1b'));
        assert!(rendered.contains("[worker A · done]"));
        assert!(rendered.contains("# Title"));
        assert!(rendered.contains("```rust\nfn main() {}\n```\n"));
        assert!(rendered.contains("end"));
    }

    #[test]
    fn render_markdown_terminal_styles_prose_and_code() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::terminal(&highlighter);
        let text = "[worker A · done]\n# Title\n\n```rust\nfn main() {}\n```\n";
        let rendered = render_markdown(&ctx, text, &skin);
        assert!(rendered.contains("[worker A · done]"));
        assert!(!rendered.contains("# Title"));
        assert!(rendered.contains("\x1b["));
        assert!(rendered.contains("```rust\n"));
        assert!(!rendered.contains("fn main() {}"));
    }

    #[test]
    fn no_color_plain_render_leaves_diffstat_uncolored() {
        // Callers set terminal=false when NO_COLOR is set or stdout is not a tty.
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::plain(&highlighter);
        let content = "\
changed in worker GUZ-628 since spawn
 link.rs | 12 ++++--
 9 files changed, 161 insertions(+), 86 deletions(-)
";
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Prose,
            content: content.to_string(),
        };
        assert_eq!(render_text_block(&ctx, &chunk, Some(&skin)), content);
    }

    #[test]
    fn terminal_render_colors_worker_diffstat_blocks() {
        let (highlighter, skin) = create_test_context();
        let ctx = RenderContext::terminal(&highlighter);
        let content = "\
changed in worker GUZ-628 since spawn
 link.rs | 12 ++++--
 9 files changed, 161 insertions(+), 86 deletions(-)
changed in worker GUZ-628 during turn 6
 b.rs | 3 ---
 1 file changed, 1 deletion(-)
";
        let chunk = ParsedChunk {
            chunk_type: ChunkType::Prose,
            content: content.to_string(),
        };
        let rendered = render_text_block(&ctx, &chunk, Some(&skin));
        assert!(rendered.contains("since spawn") && rendered.contains("during turn 6"));
        assert!(
            rendered.contains("\x1b[32m++++") && rendered.contains("\x1b[31m--"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\x1b[32m161") && rendered.contains("\x1b[31m86"),
            "{rendered}"
        );
        assert!(rendered.contains("\x1b[31m---"), "{rendered}");
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
    fn test_derive_top_border() {
        assert_eq!(derive_top_border("├───────┼──────┤"), "┌───────┬──────┐");
    }

    #[test]
    fn test_derive_bottom_border() {
        assert_eq!(derive_bottom_border("├───────┼──────┤"), "└───────┴──────┘");
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
        assert_eq!(lines[0], "┌───────┬──────┐");
        assert_eq!(lines[1], "│Feature│Status│");
        assert_eq!(lines[2], "├───────┼──────┤");
        assert_eq!(lines[3], "│Auth   │Done  │");
        assert_eq!(lines[4], "└───────┴──────┘");
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
        // Without a separator, no borders are added, cells are unchanged
        assert!(result.contains("│A│B│"));
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
