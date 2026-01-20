//! Render context and ANSI color constants.

use crate::highlight::Highlighter;

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
