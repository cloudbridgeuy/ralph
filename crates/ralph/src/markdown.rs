//! Markdown rendering for prose output (Imperative Shell).
//!
//! This module provides terminal-friendly markdown rendering using the termimad
//! library. It handles prose output from LLM responses, rendering bold, italic,
//! headers, lists, and inline code with ANSI formatting.
//!
//! # Design Decision
//!
//! We use termimad for prose/markdown rendering and syntect for code blocks:
//! - termimad: Optimized for terminal markdown (headers, lists, emphasis, inline code)
//! - syntect: Language-aware syntax highlighting for fenced code blocks
//!
//! This separation provides the best of both worlds - natural markdown rendering
//! for prose sections while preserving full syntax highlighting for code.
//!
//! # Example
//!
//! ```
//! use ralph::markdown::MarkdownRenderer;
//!
//! let renderer = MarkdownRenderer::new();
//! let rendered = renderer.render("# Hello\n\nThis is **bold** text.");
//! ```

use termimad::{MadSkin, StyledChar};

/// Create a configured `MadSkin` for terminal markdown rendering (Functional Core).
///
/// This is a pure function that produces the shared skin used across all
/// rendering paths: streaming, replay, and batch. Customize colors here
/// and every renderer picks them up automatically.
pub fn create_markdown_skin() -> MadSkin {
    use termimad::crossterm::style::{Attribute, Color};

    let mut skin = MadSkin::default();

    skin.headers[0].set_fg(Color::Cyan);
    skin.headers[0].add_attr(Attribute::Bold);

    skin.headers[1].set_fg(Color::Blue);
    skin.headers[1].add_attr(Attribute::Bold);

    skin.headers[2].set_fg(Color::Magenta);
    skin.headers[2].add_attr(Attribute::Bold);

    skin.bold.add_attr(Attribute::Bold);
    skin.italic.add_attr(Attribute::Italic);

    // Yellow on dark background for visual distinction from prose
    skin.inline_code.set_fgbg(Color::Yellow, termimad::gray(3));

    skin.bullet = StyledChar::from_fg_char(Color::DarkGrey, '•');

    skin
}

/// A markdown renderer for terminal output using termimad.
///
/// This struct wraps termimad's MadSkin with customizations suitable for
/// streaming LLM output. It handles:
/// - Headers with emphasis
/// - Bold and italic text
/// - Inline code with distinct styling
/// - Numbered and bulleted lists
/// - Links rendered visibly
#[derive(Clone)]
pub struct MarkdownRenderer {
    skin: MadSkin,
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownRenderer {
    /// Create a new markdown renderer with default terminal styling.
    ///
    /// The default skin uses:
    /// - Bold for headers (h1-h3)
    /// - Standard bold/italic for emphasis
    /// - Distinct background for inline code
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::markdown::MarkdownRenderer;
    ///
    /// let renderer = MarkdownRenderer::new();
    /// ```
    pub fn new() -> Self {
        Self {
            skin: create_markdown_skin(),
        }
    }

    /// Render markdown text to a string with ANSI formatting.
    ///
    /// This method converts markdown syntax to ANSI-escaped terminal output.
    /// Headers, bold, italic, inline code, and lists are all rendered with
    /// appropriate terminal styling.
    ///
    /// # Arguments
    ///
    /// * `text` - The markdown text to render
    ///
    /// # Returns
    ///
    /// A string containing the rendered text with ANSI escape sequences.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::markdown::MarkdownRenderer;
    ///
    /// let renderer = MarkdownRenderer::new();
    /// let rendered = renderer.render("**bold** and *italic*");
    /// // rendered contains ANSI escape sequences for bold and italic
    /// ```
    pub fn render(&self, text: &str) -> String {
        // termimad's text method renders markdown to a string with ANSI codes
        self.skin.text(text, None).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_renderer_new() {
        let renderer = MarkdownRenderer::new();
        // Should create without panic
        let _ = renderer.render("test");
    }

    #[test]
    fn test_renderer_default() {
        let renderer = MarkdownRenderer::default();
        let _ = renderer.render("test");
    }

    #[test]
    fn test_render_plain_text() {
        let renderer = MarkdownRenderer::new();
        let rendered = renderer.render("plain text");

        // Should contain the text
        assert!(rendered.contains("plain text"));
    }

    #[test]
    fn test_render_bold_text() {
        let renderer = MarkdownRenderer::new();
        let rendered = renderer.render("**bold**");

        // Should contain ANSI escape codes for bold
        assert!(rendered.contains("\x1b["));
        // Should contain the word
        assert!(rendered.contains("bold"));
    }

    #[test]
    fn test_render_italic_text() {
        let renderer = MarkdownRenderer::new();
        let rendered = renderer.render("*italic*");

        // Should contain ANSI escape codes
        assert!(rendered.contains("\x1b["));
        // Should contain the word
        assert!(rendered.contains("italic"));
    }

    #[test]
    fn test_render_inline_code() {
        let renderer = MarkdownRenderer::new();
        let rendered = renderer.render("use `code` here");

        // Should contain ANSI escape codes for inline code styling
        assert!(rendered.contains("\x1b["));
        // Should contain the code
        assert!(rendered.contains("code"));
    }

    #[test]
    fn test_render_header() {
        let renderer = MarkdownRenderer::new();
        let rendered = renderer.render("# Header");

        // Should contain ANSI escape codes for header styling
        assert!(rendered.contains("\x1b["));
        // Should contain the header text
        assert!(rendered.contains("Header"));
    }

    #[test]
    fn test_render_bulleted_list() {
        let renderer = MarkdownRenderer::new();
        let rendered = renderer.render("- item 1\n- item 2");

        // Should contain the list items
        assert!(rendered.contains("item 1"));
        assert!(rendered.contains("item 2"));
    }

    #[test]
    fn test_render_numbered_list() {
        let renderer = MarkdownRenderer::new();
        let rendered = renderer.render("1. first\n2. second");

        // Should contain the list items
        assert!(rendered.contains("first"));
        assert!(rendered.contains("second"));
    }

    #[test]
    fn test_render_nested_formatting() {
        let renderer = MarkdownRenderer::new();
        let rendered = renderer.render("This is **bold and *italic***");

        // Should render without crashing
        assert!(rendered.contains("bold"));
        assert!(rendered.contains("italic"));
    }

    #[test]
    fn test_render_empty_string() {
        let renderer = MarkdownRenderer::new();
        let rendered = renderer.render("");

        // Should handle empty input gracefully
        assert!(rendered.is_empty() || rendered.chars().all(|c| c.is_whitespace()));
    }

    #[test]
    fn test_render_multiline() {
        let renderer = MarkdownRenderer::new();
        let text = "First paragraph.\n\nSecond paragraph.";
        let rendered = renderer.render(text);

        // Should contain both paragraphs
        assert!(rendered.contains("First"));
        assert!(rendered.contains("Second"));
    }

    #[test]
    fn test_render_preserves_whitespace() {
        let renderer = MarkdownRenderer::new();
        let text = "Line with   multiple   spaces";
        let rendered = renderer.render(text);

        // termimad may normalize whitespace, but content should be preserved
        assert!(rendered.contains("Line"));
        assert!(rendered.contains("multiple"));
        assert!(rendered.contains("spaces"));
    }

    #[test]
    fn test_render_special_characters() {
        let renderer = MarkdownRenderer::new();
        let text = "Special chars: <>&\"'";
        let rendered = renderer.render(text);

        // Special characters should be preserved
        assert!(rendered.contains("<"));
        assert!(rendered.contains(">"));
        assert!(rendered.contains("&"));
    }

    #[test]
    fn test_render_unicode() {
        let renderer = MarkdownRenderer::new();
        let text = "Unicode: 你好 世界 🎉";
        let rendered = renderer.render(text);

        // Unicode should be preserved
        assert!(rendered.contains("你好"));
        assert!(rendered.contains("世界"));
        assert!(rendered.contains("🎉"));
    }

    #[test]
    fn test_create_markdown_skin_returns_configured_skin() {
        let skin = create_markdown_skin();
        // Inline code should produce ANSI escape codes (fg + bg set)
        let rendered = skin.inline("`code`").to_string();
        assert!(
            rendered.contains("\x1b["),
            "inline code should have ANSI styling"
        );
        assert!(rendered.contains("code"));
    }
}
