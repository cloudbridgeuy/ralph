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
                    .map(|skin| skin.term_text(&chunk.content).to_string())
                    .unwrap_or_else(|| chunk.content.clone())
            } else {
                chunk.content.clone()
            }
        }
        ChunkType::Code { language } => render_code_block(ctx, &chunk.content, language.as_deref()),
        ChunkType::Diff => render_diff_block(ctx, &chunk.content),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::{Highlighter, ThemeConfig};

    fn create_test_context() -> (Highlighter, MadSkin) {
        let highlighter = Highlighter::with_config(ThemeConfig::default()).unwrap();
        let skin = MadSkin::default();
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
}
