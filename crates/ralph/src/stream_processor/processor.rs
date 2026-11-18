use super::block_renderer::BlockRenderer;
use super::disk_files::DiskFiles;
use crate::highlight::{ThemeConfig, ThemeError};
use crate::render::RenderContext;
use ralph_core::stream::blocks::{
    OutputBlock, StreamCollector, StreamProcessorResult, VerboseToolsConfig,
};
use std::io::IsTerminal;

pub struct StreamProcessor {
    pub(super) collector: StreamCollector,
    pub(super) renderer: BlockRenderer,
}

/// Builder for constructing StreamProcessor instances.
///
/// Provides a fluent API for configuring all StreamProcessor options
/// with sensible defaults.
///
/// # Example
///
/// ```
/// use ralph::stream_processor::StreamProcessorBuilder;
/// use ralph::highlight::ThemeConfig;
///
/// let processor = StreamProcessorBuilder::new()
///     .highlighting(true)
///     .show_tools(true)
///     .theme_config(ThemeConfig::new().with_theme("Monokai Extended"))
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Default)]
pub struct StreamProcessorBuilder {
    highlighting: Option<bool>,
    show_tools: Option<bool>,
    theme_config: Option<ThemeConfig>,
    verbose_tools: Option<VerboseToolsConfig>,
}

impl StreamProcessorBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set whether to enable syntax highlighting.
    ///
    /// If not set, defaults to terminal detection (enabled if stdout is a terminal).
    pub fn highlighting(mut self, enabled: bool) -> Self {
        self.highlighting = Some(enabled);
        self
    }

    /// Set whether to display tool invocations.
    ///
    /// If not set, defaults to the highlighting setting.
    pub fn show_tools(mut self, enabled: bool) -> Self {
        self.show_tools = Some(enabled);
        self
    }

    /// Set the theme configuration for syntax highlighting.
    pub fn theme_config(mut self, config: ThemeConfig) -> Self {
        self.theme_config = Some(config);
        self
    }

    /// Set the verbose tools configuration.
    pub fn verbose_tools(mut self, config: VerboseToolsConfig) -> Self {
        self.verbose_tools = Some(config);
        self
    }

    /// Build the StreamProcessor.
    ///
    /// # Returns
    ///
    /// * `Ok(StreamProcessor)` - Successfully configured processor
    /// * `Err(ThemeError)` - If the theme configuration is invalid
    pub fn build(self) -> Result<StreamProcessor, ThemeError> {
        let is_terminal = std::io::stdout().is_terminal();

        // Determine highlighting: explicit setting > terminal detection
        let highlighting_enabled = self.highlighting.unwrap_or(is_terminal);

        // Determine show_tools: explicit setting > highlighting setting
        let show_tool_invocations = self.show_tools.unwrap_or(highlighting_enabled);

        let renderer = BlockRenderer::new(self.theme_config, highlighting_enabled)?;

        // Get verbose tools config
        let verbose_tools_config = self.verbose_tools.unwrap_or_default();

        Ok(StreamProcessor {
            collector: StreamCollector::new(show_tool_invocations, verbose_tools_config),
            renderer,
        })
    }
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
        StreamProcessorBuilder::new()
            .build()
            .expect("default builder should not fail")
    }

    #[allow(
        clippy::expect_used,
        reason = "building without a theme config has no error path"
    )]
    pub fn with_highlighting(enabled: bool) -> Self {
        StreamProcessorBuilder::new()
            .highlighting(enabled)
            .build()
            .expect("builder without theme config should not fail")
    }

    #[allow(
        clippy::expect_used,
        reason = "building without a theme config has no error path"
    )]
    pub fn with_options(highlighting: bool, show_tools: bool) -> Self {
        StreamProcessorBuilder::new()
            .highlighting(highlighting)
            .show_tools(show_tools)
            .build()
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
        StreamProcessorBuilder::new()
            .highlighting(highlighting)
            .show_tools(show_tools)
            .verbose_tools(verbose_tools)
            .build()
            .expect("builder without theme config should not fail")
    }

    /// Create a processor with custom theme configuration.
    ///
    /// # Arguments
    ///
    /// * `theme_config` - Configuration for syntax highlighting theme
    ///
    /// # Returns
    ///
    /// * `Ok(StreamProcessor)` - Successfully configured processor
    /// * `Err(ThemeError)` - If the theme was not found or failed to load
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::stream_processor::StreamProcessor;
    /// use ralph::highlight::ThemeConfig;
    ///
    /// let config = ThemeConfig::new().with_theme("Monokai Extended");
    /// let processor = StreamProcessor::with_theme_config(config).unwrap();
    /// ```
    pub fn with_theme_config(theme_config: ThemeConfig) -> Result<Self, ThemeError> {
        StreamProcessorBuilder::new()
            .theme_config(theme_config)
            .build()
    }

    /// Create a processor with full configuration.
    ///
    /// # Arguments
    ///
    /// * `theme_config` - Configuration for syntax highlighting theme
    /// * `highlighting` - Whether to apply syntax highlighting (overrides terminal detection)
    /// * `show_tools` - Whether to display tool invocations
    ///
    /// # Returns
    ///
    /// * `Ok(StreamProcessor)` - Successfully configured processor
    /// * `Err(ThemeError)` - If the theme was not found or failed to load
    pub fn with_full_config(
        theme_config: ThemeConfig,
        highlighting: bool,
        show_tools: bool,
    ) -> Result<Self, ThemeError> {
        StreamProcessorBuilder::new()
            .theme_config(theme_config)
            .highlighting(highlighting)
            .show_tools(show_tools)
            .build()
    }

    /// Create a processor with verbose tools configuration.
    ///
    /// # Arguments
    ///
    /// * `theme_config` - Configuration for syntax highlighting theme
    /// * `highlighting` - Whether to apply syntax highlighting
    /// * `show_tools` - Whether to display tool invocations
    /// * `verbose_tools` - Configuration for verbose tool output
    ///
    /// # Returns
    ///
    /// * `Ok(StreamProcessor)` - Successfully configured processor
    /// * `Err(ThemeError)` - If the theme was not found or failed to load
    pub fn with_verbose_tools(
        theme_config: ThemeConfig,
        highlighting: bool,
        show_tools: bool,
        verbose_tools: VerboseToolsConfig,
    ) -> Result<Self, ThemeError> {
        StreamProcessorBuilder::new()
            .theme_config(theme_config)
            .highlighting(highlighting)
            .show_tools(show_tools)
            .verbose_tools(verbose_tools)
            .build()
    }

    /// Check if highlighting is enabled.
    pub fn is_highlighting_enabled(&self) -> bool {
        self.renderer.is_highlighting_enabled()
    }

    pub fn process_line(&mut self, line: &str) -> Option<String> {
        let blocks = self.collector.process_line(line, &DiskFiles);
        self.renderer.render_visible(&blocks)
    }

    pub fn finish(self) -> StreamProcessorResult {
        let Self {
            collector,
            mut renderer,
        } = self;
        let (flushed, mut result) = collector.finish(&DiskFiles);
        result.final_output = Some(renderer.render(&flushed)).filter(|output| !output.is_empty());
        result
    }

    /// Get parse errors encountered during processing.
    pub fn parse_errors(&self) -> &[(String, String)] {
        self.collector.parse_errors()
    }

    /// Check if tool invocation display is enabled.
    pub fn is_showing_tool_invocations(&self) -> bool {
        self.collector.is_showing_tool_invocations()
    }

    /// Get the count of distinct assistant responses processed.
    ///
    /// This increments each time a new message ID is seen after a previous
    /// message has started. Useful for testing and debugging.
    pub fn response_count(&self) -> usize {
        self.collector.response_count()
    }

    /// Check if any output has been emitted.
    ///
    /// Used for visual separation logic - we only add separators between
    /// responses if there's been output to separate.
    pub fn has_emitted_output(&self) -> bool {
        self.renderer.has_emitted_output()
    }

    /// Get the verbose tools configuration.
    pub fn verbose_tools_config(&self) -> &VerboseToolsConfig {
        self.collector.verbose_tools_config()
    }

    /// Check if verbose output is enabled for a specific tool.
    pub fn is_tool_verbose(&self, tool_name: &str) -> bool {
        self.collector.verbose_tools_config().is_verbose(tool_name)
    }

    /// Create a render context based on the processor's highlighting setting.
    ///
    /// Returns a terminal context (with ANSI codes) when highlighting is enabled,
    /// or a plain context otherwise.
    pub fn render_context(&self) -> RenderContext<'_> {
        self.renderer.render_context()
    }

    /// Get the accumulated output blocks.
    ///
    /// These blocks capture the structured data behind the rendered output.
    pub fn output_blocks(&self) -> &[OutputBlock] {
        self.collector.output_blocks()
    }
}
