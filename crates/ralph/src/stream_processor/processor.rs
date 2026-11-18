use super::block_renderer::BlockRenderer;
use super::disk_files::DiskFiles;
use crate::highlight::{ThemeConfig, ThemeError};
use crate::render::RenderContext;
use ralph_core::stream::blocks::{
    OutputBlock, StreamCollector, StreamProcessorResult, VerboseToolsConfig,
};
use std::io::IsTerminal;

fn is_terminal_stdout() -> bool {
    std::io::stdout().is_terminal()
}

pub struct StreamProcessor {
    pub(super) collector: StreamCollector,
    pub(super) renderer: BlockRenderer,
}

impl Default for StreamProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamProcessor {
    #[allow(
        clippy::expect_used,
        reason = "building without a theme config has no error path"
    )]
    pub fn new() -> Self {
        Self::build(
            None,
            is_terminal_stdout(),
            None,
            VerboseToolsConfig::default(),
        )
        .expect("default configuration should not fail")
    }

    #[allow(
        clippy::expect_used,
        reason = "building without a theme config has no error path"
    )]
    pub fn with_highlighting(enabled: bool) -> Self {
        Self::build(None, enabled, None, VerboseToolsConfig::default())
            .expect("builder without theme config should not fail")
    }

    #[allow(
        clippy::expect_used,
        reason = "building without a theme config has no error path"
    )]
    pub fn with_options(highlighting: bool, show_tools: bool) -> Self {
        Self::build(
            None,
            highlighting,
            Some(show_tools),
            VerboseToolsConfig::default(),
        )
        .expect("builder without theme config should not fail")
    }

    #[allow(
        clippy::expect_used,
        reason = "building without a theme config has no error path"
    )]
    pub fn with_options_and_verbose(
        highlighting: bool,
        show_tools: bool,
        verbose_tools: VerboseToolsConfig,
    ) -> Self {
        Self::build(None, highlighting, Some(show_tools), verbose_tools)
            .expect("builder without theme config should not fail")
    }

    pub fn with_verbose_tools(
        theme_config: ThemeConfig,
        highlighting: bool,
        show_tools: bool,
        verbose_tools: VerboseToolsConfig,
    ) -> Result<Self, ThemeError> {
        Self::build(
            Some(theme_config),
            highlighting,
            Some(show_tools),
            verbose_tools,
        )
    }

    fn build(
        theme_config: Option<ThemeConfig>,
        highlighting: bool,
        show_tools: Option<bool>,
        verbose_tools: VerboseToolsConfig,
    ) -> Result<Self, ThemeError> {
        Ok(Self {
            collector: StreamCollector::new(show_tools.unwrap_or(highlighting), verbose_tools),
            renderer: BlockRenderer::new(theme_config, highlighting)?,
        })
    }

    pub fn is_highlighting_enabled(&self) -> bool {
        self.renderer.is_highlighting_enabled()
    }

    pub fn process_line(&mut self, line: &str) -> Option<String> {
        let blocks = self.collector.process_line(line, &DiskFiles);
        self.renderer.render_visible(&blocks)
    }

    pub fn finish(self) -> StreamProcessorResult {
        self.finish_rendered().0
    }

    pub fn finish_rendered(self) -> (StreamProcessorResult, String) {
        let Self {
            collector,
            mut renderer,
        } = self;
        let (flushed, result) = collector.finish(&DiskFiles);
        let rendered = renderer.render(&flushed);
        (result, rendered)
    }

    pub fn parse_errors(&self) -> &[(String, String)] {
        self.collector.parse_errors()
    }

    pub fn is_showing_tool_invocations(&self) -> bool {
        self.collector.is_showing_tool_invocations()
    }

    pub fn response_count(&self) -> usize {
        self.collector.response_count()
    }

    pub fn has_emitted_output(&self) -> bool {
        self.renderer.has_emitted_output()
    }

    pub fn verbose_tools_config(&self) -> &VerboseToolsConfig {
        self.collector.verbose_tools_config()
    }

    pub fn is_tool_verbose(&self, tool_name: &str) -> bool {
        self.collector.verbose_tools_config().is_verbose(tool_name)
    }

    pub fn render_context(&self) -> RenderContext<'_> {
        self.renderer.render_context()
    }

    pub fn output_blocks(&self) -> &[OutputBlock] {
        self.collector.output_blocks()
    }
}
