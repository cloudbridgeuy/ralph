use crate::diff_theme::DiffColors;
use crate::highlight::Highlighter;
use crate::tool_theme::ToolColors;
use crate::ui_theme::UiColors;
use ralph_core::mermaid_file::Miss;
use std::path::{Path, PathBuf};
use syntect::highlighting::Theme;

pub use crate::ansi;

pub type SaveDiagram = fn(&str, &Theme, &UiColors, &Path) -> Option<PathBuf>;

#[derive(Clone, Copy)]
pub struct Diagrams<'a> {
    pub dir: &'a Path,
    pub ui: UiColors,
    pub save: SaveDiagram,
}

#[derive(Clone, Copy)]
pub struct RenderContext<'a> {
    pub highlighter: &'a Highlighter,
    pub terminal: bool,
    pub width: usize,
    pub viewport: Viewport,
    pub mermaid: fn(&str, usize) -> Result<String, Miss>,
    pub diagrams: Option<Diagrams<'a>>,
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
            diagrams: None,
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

    pub fn refreshed(self) -> Self {
        if self.terminal {
            self.with_viewport(self.viewport)
        } else {
            self
        }
    }

    pub fn with_diff(mut self, diff: DiffColors) -> Self {
        self.diff = diff;
        self
    }

    pub fn with_diagrams(mut self, dir: &'a Path, ui: UiColors) -> Self {
        self.diagrams = Some(Diagrams {
            dir,
            ui,
            save: crate::mermaid_guard::full_size,
        });
        self
    }

    pub fn with_tools(mut self, tools: ToolColors) -> Self {
        self.tools = tools;
        self
    }

    pub fn terminal(highlighter: &'a Highlighter) -> Self {
        Self::new(highlighter, true)
    }

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
