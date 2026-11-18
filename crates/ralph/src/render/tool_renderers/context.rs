//! Render context and ANSI color constants.

use crate::diff_theme::DiffColors;
use crate::highlight::Highlighter;
use crate::tool_theme::ToolColors;

pub use crate::ansi;

pub struct RenderContext<'a> {
    pub highlighter: &'a Highlighter,
    pub terminal: bool,
    pub width: usize,
    pub viewport: Viewport,
    pub mermaid: fn(&str, usize) -> Option<String>,
    pub images: Option<crate::render::ImageSettings>,
    pub diff: DiffColors,
    pub tools: ToolColors,
}

const DEFAULT_WIDTH: usize = 80;
const TERMIMAD_FALLBACK: (u16, u16) = (50, 20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Viewport {
    Live,
    Measured(Option<(u16, u16)>),
}

impl Viewport {
    pub fn fixed(cols: u16, rows: u16) -> Self {
        Self::Measured(Some((cols, rows)))
    }

    pub fn unmeasured() -> Self {
        Self::Measured(None)
    }

    fn measured(self) -> Option<(u16, u16)> {
        match self {
            Self::Live => crossterm::terminal::size().ok(),
            Self::Measured(size) => size,
        }
    }

    pub fn termimad_size(self) -> (u16, u16) {
        self.measured().unwrap_or(TERMIMAD_FALLBACK)
    }

    fn width(self, terminal: bool) -> usize {
        if !terminal {
            return DEFAULT_WIDTH;
        }
        self.measured()
            .map_or(DEFAULT_WIDTH, |(cols, _)| usize::from(cols))
    }
}

impl<'a> RenderContext<'a> {
    pub fn new(highlighter: &'a Highlighter, terminal: bool) -> Self {
        let viewport = Viewport::Live;
        Self {
            highlighter,
            terminal,
            width: viewport.width(terminal),
            viewport,
            mermaid: crate::mermaid_guard::render,
            images: crate::render::image::enabled(),
            diff: DiffColors::default(),
            tools: ToolColors::default(),
        }
    }

    pub fn with_viewport(mut self, viewport: Viewport) -> Self {
        self.width = viewport.width(self.terminal);
        self.viewport = viewport;
        self
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::{Highlighter, ThemeConfig};

    fn width(terminal: bool, viewport: Viewport) -> usize {
        let highlighter = Highlighter::with_config(ThemeConfig::default()).unwrap();
        RenderContext::new(&highlighter, terminal)
            .with_viewport(viewport)
            .width
    }

    #[test]
    fn terminal_width_comes_from_the_viewport() {
        assert_eq!(width(true, Viewport::fixed(60, 24)), 60);
        assert_eq!(width(true, Viewport::fixed(100, 24)), 100);
    }

    #[test]
    fn width_falls_back_to_80_when_unmeasured_or_plain() {
        assert_eq!(width(true, Viewport::unmeasured()), 80);
        assert_eq!(width(false, Viewport::fixed(60, 24)), 80);
    }

    #[test]
    fn termimad_size_keeps_the_termimad_fallback() {
        assert_eq!(Viewport::fixed(60, 24).termimad_size(), (60, 24));
        assert_eq!(Viewport::unmeasured().termimad_size(), (50, 20));
    }
}
