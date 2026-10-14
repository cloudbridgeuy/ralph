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

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use termimad::crossterm::style::{Attribute, Color};
use termimad::{MadSkin, StyledChar};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeRole {
    Assistant,
    Agent,
}

impl ThemeRole {
    fn name(self) -> &'static str {
        match self {
            ThemeRole::Assistant => "assistant",
            ThemeRole::Agent => "agent",
        }
    }

    fn base(self) -> Palette {
        match self {
            ThemeRole::Assistant => Palette::default(),
            ThemeRole::Agent => Palette {
                headings: [
                    Some(Color::Green),
                    Some(Color::DarkGreen),
                    Some(Color::DarkYellow),
                ],
                inline_code: Some(Color::DarkCyan),
                bullet: Some(Color::DarkGreen),
                ..Palette::default()
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub headings: [Option<Color>; 3],
    pub bold: Option<Color>,
    pub italic: Option<Color>,
    pub inline_code: Option<Color>,
    pub bullet: Option<Color>,
    pub quote: Option<Color>,
    pub table: Option<Color>,
    pub link: Option<Color>,
    pub url: Option<Color>,
    pub memory_ref: Option<Color>,
    pub border: Option<Color>,
    pub done: Option<Color>,
    pub blocked: Option<Color>,
    pub waiting: Option<Color>,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            headings: [Some(Color::Cyan), Some(Color::Blue), Some(Color::Magenta)],
            bold: None,
            italic: None,
            inline_code: Some(Color::Yellow),
            bullet: Some(Color::DarkGrey),
            quote: None,
            table: None,
            link: Some(Color::Blue),
            url: Some(Color::Cyan),
            memory_ref: Some(Color::Magenta),
            border: Some(Color::DarkBlue),
            done: Some(Color::DarkGreen),
            blocked: Some(Color::DarkRed),
            waiting: Some(Color::DarkYellow),
        }
    }
}

impl Palette {
    fn slot(&mut self, key: &str) -> Option<&mut Option<Color>> {
        match key {
            "heading1" => Some(&mut self.headings[0]),
            "heading2" => Some(&mut self.headings[1]),
            "heading3" => Some(&mut self.headings[2]),
            "bold" => Some(&mut self.bold),
            "italic" => Some(&mut self.italic),
            "inline_code" => Some(&mut self.inline_code),
            "bullet" => Some(&mut self.bullet),
            "quote" => Some(&mut self.quote),
            "table" => Some(&mut self.table),
            "link" => Some(&mut self.link),
            "url" => Some(&mut self.url),
            "memory_ref" => Some(&mut self.memory_ref),
            "border" => Some(&mut self.border),
            "done" => Some(&mut self.done),
            "blocked" => Some(&mut self.blocked),
            "waiting" => Some(&mut self.waiting),
            _ => None,
        }
    }
}

pub fn parse_color(value: &str) -> Option<Color> {
    let Some(hex) = value.strip_prefix('#') else {
        return Color::try_from(value).ok();
    };
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some(Color::Rgb {
        r: channel(0)?,
        g: channel(2)?,
        b: channel(4)?,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoleTheme {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(flatten)]
    pub colors: BTreeMap<String, String>,
}

impl RoleTheme {
    pub fn palette(&self, role: ThemeRole) -> (Palette, Vec<String>) {
        let mut palette = role.base();
        let mut warnings = Vec::new();
        for (key, value) in &self.colors {
            match (palette.slot(key), parse_color(value)) {
                (Some(slot), Some(color)) => *slot = Some(color),
                (Some(_), None) => warnings.push(format!(
                    "[theme.{}] {key} = \"{value}\" is not a color; using the default",
                    role.name()
                )),
                (None, _) => warnings.push(format!(
                    "[theme.{}] unknown key \"{key}\"; ignored",
                    role.name()
                )),
            }
        }
        (palette, warnings)
    }
}

pub fn markdown_skin(palette: &Palette) -> MadSkin {
    let mut skin = MadSkin::default();
    for (header, color) in skin.headers.iter_mut().zip(palette.headings) {
        if let Some(color) = color {
            header.set_fg(color);
        }
        header.add_attr(Attribute::Bold);
    }
    skin.bold.add_attr(Attribute::Bold);
    skin.italic.add_attr(Attribute::Italic);
    if let Some(color) = palette.bold {
        skin.bold.set_fg(color);
    }
    if let Some(color) = palette.italic {
        skin.italic.set_fg(color);
    }
    skin.inline_code.set_bg(termimad::gray(3));
    if let Some(color) = palette.inline_code {
        skin.inline_code.set_fg(color);
    }
    skin.bullet = palette.bullet.map_or(StyledChar::nude('•'), |color| {
        StyledChar::from_fg_char(color, '•')
    });
    if let Some(color) = palette.quote {
        skin.quote_mark.set_fg(color);
    }
    if let Some(color) = palette.table {
        skin.table.set_fg(color);
    }
    skin
}

#[derive(Clone)]
pub struct MarkdownTheme {
    pub skin: MadSkin,
    pub palette: Palette,
}

impl MarkdownTheme {
    pub fn new(palette: Palette) -> Self {
        Self {
            skin: markdown_skin(&palette),
            palette,
        }
    }
}

impl Default for MarkdownTheme {
    fn default() -> Self {
        Self::new(Palette::default())
    }
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
            skin: markdown_skin(&Palette::default()),
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

    fn role_theme(pairs: &[(&str, &str)]) -> RoleTheme {
        RoleTheme {
            code: None,
            colors: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    #[test]
    fn agent_skin_differs_from_assistant_skin() {
        let assistant = markdown_skin(&ThemeRole::Assistant.base())
            .term_text("# Title")
            .to_string();
        let agent = markdown_skin(&ThemeRole::Agent.base())
            .term_text("# Title")
            .to_string();
        assert!(agent.contains("Title"));
        assert_ne!(assistant, agent);
    }

    #[test]
    fn empty_role_theme_keeps_role_defaults() {
        assert_eq!(
            RoleTheme::default().palette(ThemeRole::Assistant),
            (Palette::default(), vec![])
        );
        assert_eq!(
            RoleTheme::default().palette(ThemeRole::Agent).0.headings[0],
            Some(Color::Green)
        );
    }

    #[test]
    fn role_theme_overrides_named_and_hex_colors() {
        let (palette, warnings) = role_theme(&[
            ("heading1", "red"),
            ("link", "#ff8800"),
            ("border", "dark_magenta"),
        ])
        .palette(ThemeRole::Agent);
        assert!(warnings.is_empty());
        assert_eq!(palette.headings[0], Some(Color::Red));
        assert_eq!(
            palette.link,
            Some(Color::Rgb {
                r: 255,
                g: 136,
                b: 0
            })
        );
        assert_eq!(palette.border, Some(Color::DarkMagenta));
        assert_eq!(palette.headings[1], Some(Color::DarkGreen));
    }

    #[test]
    fn role_theme_warns_on_bad_color_and_unknown_key() {
        let (palette, warnings) = role_theme(&[("heading1", "#12345"), ("heading9", "red")])
            .palette(ThemeRole::Assistant);
        assert_eq!(palette, Palette::default());
        assert_eq!(
            warnings,
            [
                "[theme.assistant] heading1 = \"#12345\" is not a color; using the default",
                "[theme.assistant] unknown key \"heading9\"; ignored",
            ]
        );
    }

    #[test]
    fn parse_color_accepts_names_and_hex_only() {
        assert_eq!(parse_color("Dark_Cyan"), Some(Color::DarkCyan));
        assert_eq!(
            parse_color("#0a0B0c"),
            Some(Color::Rgb {
                r: 10,
                g: 11,
                b: 12
            })
        );
        assert_eq!(parse_color("#+f0000"), None);
        assert_eq!(parse_color("#ffff"), None);
        assert_eq!(parse_color("pink"), None);
    }

    #[test]
    fn skin_applies_inline_code_color() {
        let palette = Palette {
            inline_code: Some(Color::Rgb { r: 1, g: 2, b: 3 }),
            ..Palette::default()
        };
        let rendered = markdown_skin(&palette).inline("`x`").to_string();
        assert!(rendered.contains("38;2;1;2;3"));
    }

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
    fn test_markdown_skin_returns_configured_skin() {
        let skin = markdown_skin(&Palette::default());
        // Inline code should produce ANSI escape codes (fg + bg set)
        let rendered = skin.inline("`code`").to_string();
        assert!(
            rendered.contains("\x1b["),
            "inline code should have ANSI styling"
        );
        assert!(rendered.contains("code"));
    }
}
