//! Core StreamProcessor struct and configuration methods.
//!
//! This module contains the main StreamProcessor type along with all
//! constructor methods and accessors. The actual event processing logic
//! is in the event_handler module.

use crate::highlight::{Highlighter, ThemeConfig, ThemeError};
use crate::markdown::create_markdown_skin;
use ralph_core::chunk::{ParsedChunk, StreamingChunkBuffer};
use ralph_core::stream::{
    correlate_tool_interactions, extract_costs_from_events_or_default,
    extract_metadata_from_events_or_default, StreamEvent, ToolCorrelator, ToolInvocation,
};
use std::collections::HashMap;
use std::io::IsTerminal;
use termimad::MadSkin;

use super::output_block::OutputBlock;
use super::types::{
    EditSnapshot, NotebookSnapshot, StreamProcessorResult, VerboseToolsConfig, WriteSnapshot,
};

/// A streaming processor for Claude's stream-json output.
///
/// This processor handles real-time parsing, highlighting, and output of
/// LLM responses. It maintains state for:
/// - JSON event parsing
/// - Text accumulation across streaming events
/// - Chunk detection (prose/code/diff boundaries)
/// - Syntax highlighting
/// - Metadata extraction
/// - Tool invocation display
/// - Visual separation between distinct assistant responses
pub struct StreamProcessor {
    /// Collected stream events for post-processing.
    pub(super) events: Vec<StreamEvent>,
    /// Accumulated text from assistant events.
    pub(super) text_buffer: String,
    /// Chunk buffer for streaming output.
    pub(super) chunk_buffer: StreamingChunkBuffer,
    /// Syntax highlighter for code blocks.
    pub(super) code_highlighter: Highlighter,
    /// Markdown skin for prose rendering via termimad.
    pub(super) markdown_skin: MadSkin,
    /// Whether highlighting is enabled (terminal detection).
    pub(super) highlighting_enabled: bool,
    /// Whether to display tool invocations.
    pub(super) show_tool_invocations: bool,
    /// Current message ID for accumulation.
    pub(super) current_message_id: Option<String>,
    /// Chunks collected during streaming.
    pub(super) collected_chunks: Vec<ParsedChunk>,
    /// Parse errors encountered.
    pub(super) parse_errors: Vec<(String, String)>,
    /// Tool correlator for tracking tool calls and results.
    pub(super) tool_correlator: ToolCorrelator,
    /// Whether we've emitted any output (for visual separation).
    pub(super) has_emitted_output: bool,
    /// Count of distinct assistant responses processed.
    pub(super) response_count: usize,
    /// Pending tool invocations keyed by tool_use_id (for special result formatting).
    pub(super) pending_invocations: HashMap<String, ToolInvocation>,
    /// Verbose tools configuration.
    pub(super) verbose_tools_config: VerboseToolsConfig,
    /// Pending Edit tool snapshots keyed by tool_use_id.
    ///
    /// When an Edit tool invocation is detected, we capture the file content
    /// before the edit runs. When the result arrives, we generate a diff
    /// by comparing the snapshot with the current file content.
    pub(super) pending_edit_snapshots: HashMap<String, EditSnapshot>,
    /// Pending Write tool snapshots keyed by tool_use_id.
    ///
    /// When a Write tool invocation is detected, we capture the file content
    /// (if it exists) before the write runs. When the result arrives, we
    /// generate a diff showing what changed or that a new file was created.
    pub(super) pending_write_snapshots: HashMap<String, WriteSnapshot>,
    /// Pending NotebookEdit tool snapshots keyed by tool_use_id.
    ///
    /// When a NotebookEdit tool invocation is detected, we capture the cell content
    /// before the edit runs. When the result arrives, we generate a diff showing
    /// what changed in the cell.
    pub(super) pending_notebook_snapshots: HashMap<String, NotebookSnapshot>,
    /// Accumulated output blocks for replay serialization.
    ///
    /// Each output block captures the data needed to re-render a piece of output.
    /// Blocks are accumulated in order as they render to stdout during execution.
    pub(super) output_blocks: Vec<OutputBlock>,
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

        // Build highlighter
        let code_highlighter = match self.theme_config {
            Some(config) => Highlighter::with_config(config)?,
            None => Highlighter::new(),
        };

        // Get verbose tools config
        let verbose_tools_config = self.verbose_tools.unwrap_or_default();

        Ok(StreamProcessor {
            events: Vec::new(),
            text_buffer: String::new(),
            chunk_buffer: StreamingChunkBuffer::with_prose_threshold(usize::MAX),
            code_highlighter,
            markdown_skin: create_markdown_skin(),
            highlighting_enabled,
            show_tool_invocations,
            current_message_id: None,
            collected_chunks: Vec::new(),
            parse_errors: Vec::new(),
            tool_correlator: ToolCorrelator::new(),
            has_emitted_output: false,
            response_count: 0,
            pending_invocations: HashMap::new(),
            verbose_tools_config,
            pending_edit_snapshots: HashMap::new(),
            pending_write_snapshots: HashMap::new(),
            pending_notebook_snapshots: HashMap::new(),
            output_blocks: Vec::new(),
        })
    }
}

impl Default for StreamProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamProcessor {
    /// Create a new stream processor.
    ///
    /// Automatically detects terminal support for highlighting.
    /// Tool invocations are displayed by default when highlighting is enabled.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::stream_processor::StreamProcessor;
    ///
    /// let processor = StreamProcessor::new();
    /// ```
    #[allow(clippy::expect_used)] // Builder with defaults cannot fail
    pub fn new() -> Self {
        StreamProcessorBuilder::new()
            .build()
            .expect("default builder should not fail")
    }

    /// Create a processor with highlighting explicitly enabled/disabled.
    ///
    /// Useful for testing or when output will be displayed later.
    /// Tool invocations display follows the highlighting setting.
    #[allow(clippy::expect_used)] // Builder with no theme config cannot fail
    pub fn with_highlighting(enabled: bool) -> Self {
        StreamProcessorBuilder::new()
            .highlighting(enabled)
            .build()
            .expect("builder without theme config should not fail")
    }

    /// Create a processor with custom settings.
    ///
    /// # Arguments
    ///
    /// * `highlighting` - Whether to apply syntax highlighting
    /// * `show_tools` - Whether to display tool invocations
    #[allow(clippy::expect_used)] // Builder with no theme config cannot fail
    pub fn with_options(highlighting: bool, show_tools: bool) -> Self {
        StreamProcessorBuilder::new()
            .highlighting(highlighting)
            .show_tools(show_tools)
            .build()
            .expect("builder without theme config should not fail")
    }

    /// Create a processor with custom settings and verbose tools configuration.
    ///
    /// # Arguments
    ///
    /// * `highlighting` - Whether to apply syntax highlighting
    /// * `show_tools` - Whether to display tool invocations
    /// * `verbose_tools` - Configuration for verbose tool output
    #[allow(clippy::expect_used)] // Builder with no theme config cannot fail
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
        self.highlighting_enabled
    }

    /// Finish processing and return the complete result.
    ///
    /// This method:
    /// 1. Flushes any remaining buffered content
    /// 2. Extracts metadata from system init event
    /// 3. Extracts costs from result event
    /// 4. Correlates tool calls with results
    /// 5. Returns all collected data
    ///
    /// # Returns
    ///
    /// A `StreamProcessorResult` containing all extracted data.
    pub fn finish(mut self) -> StreamProcessorResult {
        // Flush remaining buffered content (renders + outputs prose/code blocks)
        let final_output = self.flush_pending_chunks();

        // Extract metadata and costs from events
        let metadata = extract_metadata_from_events_or_default(&self.events);
        let costs = extract_costs_from_events_or_default(&self.events);

        // Correlate tool interactions
        let tool_interactions = correlate_tool_interactions(&self.events);

        StreamProcessorResult {
            chunks: self.collected_chunks,
            metadata,
            costs,
            tool_interactions,
            raw_text: self.text_buffer,
            output_blocks: self.output_blocks,
            final_output,
        }
    }

    /// Get the raw accumulated text (for completion marker detection).
    pub fn raw_text(&self) -> &str {
        &self.text_buffer
    }

    /// Get parse errors encountered during processing.
    pub fn parse_errors(&self) -> &[(String, String)] {
        &self.parse_errors
    }

    /// Check if tool invocation display is enabled.
    pub fn is_showing_tool_invocations(&self) -> bool {
        self.show_tool_invocations
    }

    /// Get the count of distinct assistant responses processed.
    ///
    /// This increments each time a new message ID is seen after a previous
    /// message has started. Useful for testing and debugging.
    pub fn response_count(&self) -> usize {
        self.response_count
    }

    /// Check if any output has been emitted.
    ///
    /// Used for visual separation logic - we only add separators between
    /// responses if there's been output to separate.
    pub fn has_emitted_output(&self) -> bool {
        self.has_emitted_output
    }

    /// Get the verbose tools configuration.
    pub fn verbose_tools_config(&self) -> &VerboseToolsConfig {
        &self.verbose_tools_config
    }

    /// Check if verbose output is enabled for a specific tool.
    pub fn is_tool_verbose(&self, tool_name: &str) -> bool {
        self.verbose_tools_config.is_verbose(tool_name)
    }

    /// Create a render context based on the processor's highlighting setting.
    ///
    /// Returns a terminal context (with ANSI codes) when highlighting is enabled,
    /// or a plain context otherwise.
    pub fn render_context(&self) -> crate::render::RenderContext<'_> {
        if self.highlighting_enabled {
            crate::render::RenderContext::terminal(&self.code_highlighter)
        } else {
            crate::render::RenderContext::plain(&self.code_highlighter)
        }
    }

    /// Get the accumulated output blocks.
    ///
    /// These blocks capture all output data needed for replay serialization.
    pub fn output_blocks(&self) -> &[OutputBlock] {
        &self.output_blocks
    }
}
