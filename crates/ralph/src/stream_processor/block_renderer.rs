use super::render_block::render_block;
use crate::diff_theme::DiffColors;
use crate::highlight::{Highlighter, ThemeConfig, ThemeError};
use crate::markdown::MarkdownTheme;
use crate::render::RenderContext;
use crate::tool_theme::ToolColors;
use ralph_core::stream::blocks::OutputBlock;

pub struct BlockRenderer {
    pub(super) code_highlighter: Highlighter,
    pub(super) markdown: MarkdownTheme,
    pub(super) diff_colors: DiffColors,
    pub(super) tool_colors: ToolColors,
    pub(super) highlighting_enabled: bool,
    has_emitted_output: bool,
}

fn counts(block: &OutputBlock, rendered: &str) -> bool {
    !matches!(block, OutputBlock::Text(_)) || !rendered.is_empty()
}

impl BlockRenderer {
    pub fn new(
        theme_config: Option<ThemeConfig>,
        highlighting_enabled: bool,
    ) -> Result<Self, ThemeError> {
        let markdown = MarkdownTheme::new(
            theme_config
                .as_ref()
                .map(|config| config.palette().clone())
                .unwrap_or_default(),
        );
        let diff_colors = theme_config
            .as_ref()
            .map(|config| *config.diff())
            .unwrap_or_default();
        let tool_colors = theme_config
            .as_ref()
            .map(|config| *config.tools())
            .unwrap_or_default();
        let code_highlighter = match theme_config {
            Some(config) => Highlighter::with_config(config)?,
            None => Highlighter::new(),
        };
        Ok(Self {
            code_highlighter,
            markdown,
            diff_colors,
            tool_colors,
            highlighting_enabled,
            has_emitted_output: false,
        })
    }

    pub fn is_highlighting_enabled(&self) -> bool {
        self.highlighting_enabled
    }

    pub fn has_emitted_output(&self) -> bool {
        self.has_emitted_output
    }

    pub fn render_context(&self) -> RenderContext<'_> {
        RenderContext::new(&self.code_highlighter, self.highlighting_enabled)
            .with_diff(self.diff_colors)
            .with_tools(self.tool_colors)
    }

    pub fn render(&mut self, blocks: &[OutputBlock]) -> String {
        self.render_counted(blocks).0
    }

    pub fn render_visible(&mut self, blocks: &[OutputBlock]) -> Option<String> {
        let (rendered, any_counted) = self.render_counted(blocks);
        any_counted.then_some(rendered)
    }

    fn render_counted(&mut self, blocks: &[OutputBlock]) -> (String, bool) {
        let ctx = self.render_context();
        let markdown = self.highlighting_enabled.then_some(&self.markdown);
        let mut rendered = String::new();
        let mut any_counted = false;
        let mut emitted = self.has_emitted_output;
        for block in blocks {
            let output = match block {
                OutputBlock::Separator if !emitted => String::new(),
                _ => render_block(&ctx, block, markdown),
            };
            let counted = match block {
                OutputBlock::Separator => emitted,
                _ => counts(block, &output),
            };
            rendered.push_str(&output);
            any_counted |= counted;
            emitted |= counted;
        }
        self.has_emitted_output = emitted;
        (rendered, any_counted)
    }
}
