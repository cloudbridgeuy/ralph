//! Render context and ANSI color constants.

use crate::diff_theme::DiffColors;
use crate::highlight::Highlighter;
use crate::tool_theme::ToolColors;

pub use crate::ansi;

/// Configuration for rendering operations.
///
/// Holds shared state needed for rendering, including the code highlighter
/// and whether terminal features (ANSI codes) are enabled.
pub struct RenderContext<'a> {
    /// Code highlighter for syntax highlighting
    pub highlighter: &'a Highlighter,
    /// Whether ANSI color codes should be included
    pub terminal: bool,
    pub width: usize,
    pub mermaid: fn(&str, usize) -> Option<String>,
    pub images: Option<crate::render::ImageSettings>,
    pub diff: DiffColors,
    pub tools: ToolColors,
}

const DEFAULT_WIDTH: usize = 80;

impl<'a> RenderContext<'a> {
    /// Create a context with explicit terminal flag.
    ///
    /// Use this when you have a boolean flag for terminal mode.
    /// For convenience, `terminal()` and `plain()` methods are also available.
    pub fn new(highlighter: &'a Highlighter, terminal: bool) -> Self {
        let width = if terminal {
            crossterm::terminal::size()
                .map(|(w, _)| usize::from(w))
                .unwrap_or(DEFAULT_WIDTH)
        } else {
            DEFAULT_WIDTH
        };
        Self {
            highlighter,
            terminal,
            width,
            mermaid: crate::mermaid_guard::render,
            images: crate::render::image::enabled(),
            diff: DiffColors::default(),
            tools: ToolColors::default(),
        }
    }

    pub fn with_diff(mut self, diff: DiffColors) -> Self {
        self.diff = diff;
        self
    }

    pub fn with_tools(mut self, tools: ToolColors) -> Self {
        self.tools = tools;
        self
    }

    /// Create a context for terminal rendering (with ANSI codes).
    pub fn terminal(highlighter: &'a Highlighter) -> Self {
        Self::new(highlighter, true)
    }

    /// Create a context for plain text rendering (no ANSI codes).
    pub fn plain(highlighter: &'a Highlighter) -> Self {
        Self::new(highlighter, false)
    }
}
