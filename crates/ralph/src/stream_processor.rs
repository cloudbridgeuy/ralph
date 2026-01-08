//! Streaming output processor for LLM subprocess output (Imperative Shell).
//!
//! This module provides real-time parsing and highlighting of Claude's
//! `--output-format stream-json` output. It parses JSON events line by line,
//! extracts text content, applies syntax highlighting to code blocks, and
//! outputs to the terminal while capturing data for storage.
//!
//! # Features
//!
//! - Real-time JSON parsing of stream-json events
//! - Syntax highlighting for code blocks using syntect
//! - Diff highlighting with delta fallback chain
//! - Terminal detection for automatic color support
//! - Metadata and tool call extraction for iteration logs
//!
//! # Example
//!
//! ```no_run
//! use ralph::stream_processor::StreamProcessor;
//!
//! let mut processor = StreamProcessor::new();
//! processor.process_line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}"#);
//! let result = processor.finish();
//! ```

use crate::diff_highlight::{highlight_with_basic_colors, DiffHighlighter};
use crate::highlight::{Highlighter, ThemeConfig, ThemeError};
use crate::markdown::MarkdownRenderer;
use ralph_core::chunk::{
    is_unfenced_diff, split_lines_preserve_trailing, ChunkType, ParsedChunk, StreamingChunkBuffer,
};
use ralph_core::stream::{
    correlate_tool_interactions, extract_costs_from_events_or_default,
    extract_metadata_from_events_or_default, parse_stream_line, IterationCosts, IterationMetadata,
    ParsedLine, StreamEvent, ToolCorrelator, ToolInteraction, ToolInvocation,
};
use serde_json::Value;
use std::collections::HashMap;
use std::io::IsTerminal;

/// Result of processing a complete stream.
#[derive(Debug, Default)]
pub struct StreamProcessorResult {
    /// Parsed chunks from assistant output (prose, code, diff).
    pub chunks: Vec<ParsedChunk>,
    /// Metadata extracted from system init event.
    pub metadata: IterationMetadata,
    /// Costs extracted from result event.
    pub costs: IterationCosts,
    /// Tool interactions (calls correlated with results).
    pub tool_interactions: Vec<ToolInteraction>,
    /// Raw accumulated text (for completion marker detection).
    pub raw_text: String,
}

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
    events: Vec<StreamEvent>,
    /// Accumulated text from assistant events.
    text_buffer: String,
    /// Chunk buffer for streaming output.
    chunk_buffer: StreamingChunkBuffer,
    /// Syntax highlighter for code blocks.
    code_highlighter: Highlighter,
    /// Diff highlighter (cached for efficiency).
    #[allow(dead_code)]
    diff_highlighter: DiffHighlighter,
    /// Markdown renderer for prose output.
    markdown_renderer: MarkdownRenderer,
    /// Whether highlighting is enabled (terminal detection).
    highlighting_enabled: bool,
    /// Whether to display tool invocations.
    show_tool_invocations: bool,
    /// Current message ID for accumulation.
    current_message_id: Option<String>,
    /// Chunks collected during streaming.
    collected_chunks: Vec<ParsedChunk>,
    /// Parse errors encountered.
    parse_errors: Vec<(String, String)>,
    /// Tool correlator for tracking tool calls and results.
    tool_correlator: ToolCorrelator,
    /// Whether we've emitted any output (for visual separation).
    has_emitted_output: bool,
    /// Count of distinct assistant responses processed.
    response_count: usize,
    /// Pending tool invocations keyed by tool_use_id (for special result formatting).
    pending_invocations: HashMap<String, ToolInvocation>,
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
    pub fn new() -> Self {
        let is_terminal = std::io::stdout().is_terminal();
        Self {
            events: Vec::new(),
            text_buffer: String::new(),
            chunk_buffer: StreamingChunkBuffer::new(),
            code_highlighter: Highlighter::new(),
            diff_highlighter: DiffHighlighter::new(),
            markdown_renderer: MarkdownRenderer::new(),
            highlighting_enabled: is_terminal,
            show_tool_invocations: is_terminal,
            current_message_id: None,
            collected_chunks: Vec::new(),
            parse_errors: Vec::new(),
            tool_correlator: ToolCorrelator::new(),
            has_emitted_output: false,
            response_count: 0,
            pending_invocations: HashMap::new(),
        }
    }

    /// Create a processor with highlighting explicitly enabled/disabled.
    ///
    /// Useful for testing or when output will be displayed later.
    /// Tool invocations display follows the highlighting setting.
    pub fn with_highlighting(enabled: bool) -> Self {
        Self {
            highlighting_enabled: enabled,
            show_tool_invocations: enabled,
            ..Self::new()
        }
    }

    /// Create a processor with custom settings.
    ///
    /// # Arguments
    ///
    /// * `highlighting` - Whether to apply syntax highlighting
    /// * `show_tools` - Whether to display tool invocations
    pub fn with_options(highlighting: bool, show_tools: bool) -> Self {
        Self {
            highlighting_enabled: highlighting,
            show_tool_invocations: show_tools,
            ..Self::new()
        }
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
        let is_terminal = std::io::stdout().is_terminal();
        let highlighter = Highlighter::with_config(theme_config)?;

        Ok(Self {
            events: Vec::new(),
            text_buffer: String::new(),
            chunk_buffer: StreamingChunkBuffer::new(),
            code_highlighter: highlighter,
            diff_highlighter: DiffHighlighter::new(),
            markdown_renderer: MarkdownRenderer::new(),
            highlighting_enabled: is_terminal,
            show_tool_invocations: is_terminal,
            current_message_id: None,
            collected_chunks: Vec::new(),
            parse_errors: Vec::new(),
            tool_correlator: ToolCorrelator::new(),
            has_emitted_output: false,
            response_count: 0,
            pending_invocations: HashMap::new(),
        })
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
        let highlighter = Highlighter::with_config(theme_config)?;

        Ok(Self {
            events: Vec::new(),
            text_buffer: String::new(),
            chunk_buffer: StreamingChunkBuffer::new(),
            code_highlighter: highlighter,
            diff_highlighter: DiffHighlighter::new(),
            markdown_renderer: MarkdownRenderer::new(),
            highlighting_enabled: highlighting,
            show_tool_invocations: show_tools,
            current_message_id: None,
            collected_chunks: Vec::new(),
            parse_errors: Vec::new(),
            tool_correlator: ToolCorrelator::new(),
            has_emitted_output: false,
            response_count: 0,
            pending_invocations: HashMap::new(),
        })
    }

    /// Check if highlighting is enabled.
    pub fn is_highlighting_enabled(&self) -> bool {
        self.highlighting_enabled
    }

    /// Process a single line of stream-json output.
    ///
    /// This method:
    /// 1. Parses the JSON line into a StreamEvent
    /// 2. Extracts text from assistant events
    /// 3. Detects chunk boundaries and applies highlighting
    /// 4. Outputs highlighted content to stdout
    /// 5. Collects data for later storage
    ///
    /// # Arguments
    ///
    /// * `line` - A single line of JSON from stream-json output
    ///
    /// # Returns
    ///
    /// Any output text that should be printed (already highlighted if applicable).
    pub fn process_line(&mut self, line: &str) -> Option<String> {
        // Skip empty lines
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        // Parse the JSON line
        match parse_stream_line(trimmed) {
            ParsedLine::Event(event) => self.handle_event(event),
            ParsedLine::Empty => None,
            ParsedLine::Error { line: _, error } => {
                self.parse_errors.push((trimmed.to_string(), error));
                None
            }
        }
    }

    /// Handle a parsed stream event.
    fn handle_event(&mut self, event: StreamEvent) -> Option<String> {
        let mut output_parts: Vec<String> = Vec::new();

        match &event {
            StreamEvent::Assistant(assistant_event) => {
                // Check if this is a new message
                let new_message_id = assistant_event.message.id.clone();
                let is_new_response =
                    new_message_id != self.current_message_id && self.current_message_id.is_some();

                if is_new_response {
                    // New message - flush any pending content
                    if let Some(flushed) = self.flush_pending_chunks() {
                        output_parts.push(flushed);
                    }
                    // Insert visual separator between distinct assistant responses
                    // (but only if we've already emitted some output)
                    if self.has_emitted_output {
                        output_parts.push("\n".to_string());
                    }
                    self.response_count += 1;
                } else if self.current_message_id.is_none() {
                    // First response
                    self.response_count = 1;
                }
                self.current_message_id = new_message_id;

                // Display tool invocations if enabled, and track them for result formatting
                let tool_invocations = assistant_event.extract_tool_invocations();
                for invocation in &tool_invocations {
                    // Track pending invocations for special result formatting
                    self.pending_invocations
                        .insert(invocation.id.clone(), invocation.clone());

                    if self.show_tool_invocations {
                        let formatted = self.format_tool_invocation(invocation);
                        output_parts.push(formatted);
                    }
                }

                // Extract text from this event
                let text = assistant_event.extract_text();
                if !text.is_empty() {
                    self.text_buffer.push_str(&text);
                    // Process through chunk buffer and output
                    if let Some(text_output) = self.process_text_for_output(&text) {
                        output_parts.push(text_output);
                    }
                }
            }
            StreamEvent::User(user_event) => {
                // Display tool results if enabled
                if self.show_tool_invocations {
                    for result in &user_event.message.content {
                        // Look up the original invocation to get the tool name
                        let invocation = result
                            .tool_use_id
                            .as_ref()
                            .and_then(|id| self.pending_invocations.remove(id));

                        let formatted = self.format_tool_result_with_context(result, invocation);
                        output_parts.push(formatted);
                    }
                }
            }
            StreamEvent::System(_) | StreamEvent::Result(_) => {
                // System and result events don't produce visible output
            }
        }

        // Track tool calls/results through correlator
        self.tool_correlator.process_event(&event);

        // Store event for post-processing
        self.events.push(event);

        if output_parts.is_empty() {
            None
        } else {
            // Mark that we've emitted output (for visual separation tracking)
            self.has_emitted_output = true;
            Some(output_parts.join(""))
        }
    }

    /// Format a tool invocation for display.
    ///
    /// File paths are shown in full without truncation for tools like Read, Edit,
    /// Write, Glob, and Grep. Other arguments (like Bash commands or prompts)
    /// are truncated to keep output readable.
    ///
    /// Bash tool invocations receive special treatment: the command is shown in
    /// full with shell syntax highlighting applied.
    fn format_tool_invocation(&self, invocation: &ralph_core::stream::ToolInvocation) -> String {
        // Special handling for Bash tool invocations
        if invocation.name == "Bash" {
            return self.format_bash_tool_invocation(invocation);
        }

        let key_arg = extract_key_argument(&invocation.name, &invocation.input);

        // Format the argument: paths shown in full, other args truncated
        let formatted_arg = key_arg.map(|arg| {
            if arg.is_path {
                arg.value
            } else {
                truncate_string(&arg.value, 60)
            }
        });

        if self.highlighting_enabled {
            // Use colors for terminal display
            format!(
                "\x1b[36m▶ {}\x1b[0m{}\n",
                invocation.name,
                if let Some(arg) = formatted_arg {
                    format!(" \x1b[90m{}\x1b[0m", arg)
                } else {
                    String::new()
                }
            )
        } else {
            // Plain text for non-terminal
            format!(
                "> {} {}\n",
                invocation.name,
                formatted_arg.unwrap_or_default()
            )
        }
    }

    /// Format a Bash tool invocation with syntax highlighting.
    ///
    /// The command is shown in full (not truncated) with shell syntax highlighting
    /// applied. Multi-line commands are displayed with proper formatting.
    fn format_bash_tool_invocation(
        &self,
        invocation: &ralph_core::stream::ToolInvocation,
    ) -> String {
        // Extract the command from the input
        let command = invocation
            .input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Check if this is a multi-line command
        let is_multiline = command.contains('\n');

        if self.highlighting_enabled {
            let mut output = String::new();

            // Header with tool name
            output.push_str("\x1b[36m▶ Bash\x1b[0m\n");

            if is_multiline {
                // Multi-line: wrap in a code block with shell highlighting
                output.push_str("```sh\n");
                let highlighted = self.code_highlighter.highlight(command, Some("sh"));
                output.push_str(&highlighted);
                if !highlighted.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("```\n");
            } else {
                // Single-line: show inline with highlighting
                output.push_str("  ");
                let highlighted = self.code_highlighter.highlight(command, Some("sh"));
                // Remove trailing reset if present to add our own formatting
                let trimmed = highlighted.trim_end_matches("\x1b[0m");
                output.push_str(trimmed);
                output.push_str("\x1b[0m\n");
            }

            output
        } else {
            // Plain text for non-terminal
            if is_multiline {
                let mut output = String::new();
                output.push_str("> Bash\n");
                output.push_str("```sh\n");
                output.push_str(command);
                if !command.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("```\n");
                output
            } else {
                format!("> Bash\n  {}\n", command)
            }
        }
    }

    /// Format a tool result for display (without invocation context).
    #[allow(dead_code)]
    fn format_tool_result(&self, result: &ralph_core::stream::ToolResult) -> String {
        self.format_tool_result_with_context(result, None)
    }

    /// Format a tool result for display with optional context from the original invocation.
    ///
    /// When the original invocation is available and the tool is "Edit", this method
    /// will detect if the result contains a diff and apply syntax highlighting.
    /// When the tool is "Bash", the output is shown with distinct styling.
    fn format_tool_result_with_context(
        &self,
        result: &ralph_core::stream::ToolResult,
        invocation: Option<ToolInvocation>,
    ) -> String {
        // Check for tool-specific formatting
        if let Some(ref inv) = invocation {
            // Edit tool with diff content
            if inv.name == "Edit" && !result.is_error {
                if let Some(ref content) = result.content {
                    // Check if content looks like a diff
                    if is_unfenced_diff(content) {
                        return self.format_edit_diff_result(inv.clone(), content);
                    }
                }
            }
            // Bash tool with output
            if inv.name == "Bash" {
                return self.format_bash_tool_result(result);
            }
        }

        // Default formatting for other tools
        let truncated_content = result
            .content
            .as_ref()
            .map(|c| truncate_string(c, 200))
            .unwrap_or_else(|| "(no output)".to_string());

        if self.highlighting_enabled {
            if result.is_error {
                // Red for errors
                format!("\x1b[31m✗ Error:\x1b[0m {}\n", truncated_content)
            } else {
                // Green check for success (dim output)
                format!("\x1b[32m✓\x1b[0m \x1b[90m{}\x1b[0m\n", truncated_content)
            }
        } else {
            // Plain text for non-terminal
            if result.is_error {
                format!("! Error: {}\n", truncated_content)
            } else {
                format!("  {}\n", truncated_content)
            }
        }
    }

    /// Format a Bash tool result with distinct output styling.
    ///
    /// The output is shown in a dimmed/muted color to distinguish it from the command.
    /// Exit code is shown if non-zero (error indicator).
    /// Very long outputs are truncated with a '... N more lines' indicator.
    fn format_bash_tool_result(&self, result: &ralph_core::stream::ToolResult) -> String {
        const MAX_OUTPUT_LINES: usize = 30;

        if result.is_error {
            // Error case - show error message with red indicator
            let error_content = result
                .content
                .as_ref()
                .map(|c| truncate_multiline(c, MAX_OUTPUT_LINES))
                .unwrap_or_else(|| ("(command failed)".to_string(), false));

            if self.highlighting_enabled {
                let mut output = String::new();
                output.push_str("\x1b[31m✗ Exit code: non-zero\x1b[0m\n");
                if !error_content.0.is_empty() {
                    output.push_str("\x1b[90m");
                    output.push_str(&error_content.0);
                    output.push_str("\x1b[0m");
                    if !error_content.0.ends_with('\n') {
                        output.push('\n');
                    }
                }
                if error_content.1 {
                    output.push_str("\x1b[90m... (output truncated)\x1b[0m\n");
                }
                output
            } else {
                let mut output = String::new();
                output.push_str("! Exit code: non-zero\n");
                if !error_content.0.is_empty() {
                    output.push_str(&error_content.0);
                    if !error_content.0.ends_with('\n') {
                        output.push('\n');
                    }
                }
                if error_content.1 {
                    output.push_str("... (output truncated)\n");
                }
                output
            }
        } else {
            // Success case - show output in dimmed style
            let content = result.content.as_deref().unwrap_or("");

            // Don't show anything for empty output
            if content.is_empty() {
                return if self.highlighting_enabled {
                    "\x1b[32m✓\x1b[0m\n".to_string()
                } else {
                    "(ok)\n".to_string()
                };
            }

            let (display_content, truncated) = truncate_multiline(content, MAX_OUTPUT_LINES);

            if self.highlighting_enabled {
                let mut output = String::new();
                output.push_str("\x1b[90m");
                output.push_str(&display_content);
                output.push_str("\x1b[0m");
                if !display_content.ends_with('\n') {
                    output.push('\n');
                }
                if truncated {
                    output.push_str("\x1b[90m... (output truncated)\x1b[0m\n");
                }
                output
            } else {
                let mut output = display_content.clone();
                if !output.ends_with('\n') {
                    output.push('\n');
                }
                if truncated {
                    output.push_str("... (output truncated)\n");
                }
                output
            }
        }
    }

    /// Format an Edit tool result that contains a diff with syntax highlighting.
    ///
    /// This displays:
    /// 1. A file path header showing which file was edited
    /// 2. The diff content with syntax highlighting (green for additions, red for deletions)
    /// 3. Truncation indicator if the diff is very long
    fn format_edit_diff_result(&self, invocation: ToolInvocation, diff_content: &str) -> String {
        // Extract file path from the invocation input
        let file_path = invocation
            .input
            .get("file_path")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown file");

        // Count lines for potential truncation
        let lines: Vec<&str> = diff_content.lines().collect();
        let line_count = lines.len();
        const MAX_DIFF_LINES: usize = 50;

        // Truncate if too long
        let (display_content, truncated) = if line_count > MAX_DIFF_LINES {
            let truncated_lines: String = lines[..MAX_DIFF_LINES].join("\n");
            (truncated_lines, true)
        } else {
            (diff_content.to_string(), false)
        };

        if self.highlighting_enabled {
            // Highlight the diff
            let highlighted_diff = highlight_with_basic_colors(&display_content);

            // Build output with header
            let mut output = String::new();

            // File path header with box drawing
            output.push_str(&format!("\x1b[36m── {} ──\x1b[0m\n", file_path));

            // The highlighted diff content wrapped in diff fences
            output.push_str("```diff\n");
            output.push_str(&highlighted_diff);
            if !highlighted_diff.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("```\n");

            // Truncation indicator
            if truncated {
                output.push_str(&format!(
                    "\x1b[90m... {} more lines\x1b[0m\n",
                    line_count - MAX_DIFF_LINES
                ));
            }

            output
        } else {
            // Plain text format
            let mut output = String::new();

            // Simple header
            output.push_str(&format!("-- {} --\n", file_path));

            // Plain diff content
            output.push_str("```diff\n");
            output.push_str(&display_content);
            if !display_content.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("```\n");

            // Truncation indicator
            if truncated {
                output.push_str(&format!("... {} more lines\n", line_count - MAX_DIFF_LINES));
            }

            output
        }
    }

    /// Process text through the chunk buffer and generate highlighted output.
    fn process_text_for_output(&mut self, text: &str) -> Option<String> {
        let mut output = String::new();

        // Process line by line through chunk buffer
        // Use split_lines_preserve_trailing to handle trailing newlines correctly
        for line in split_lines_preserve_trailing(text) {
            let chunks = self.chunk_buffer.process_line(line);

            for chunk in chunks {
                // Store chunk
                self.collected_chunks.push(chunk.clone());

                // Generate highlighted output
                let highlighted = self.highlight_chunk(&chunk);
                output.push_str(&highlighted);
                output.push('\n');
            }
        }

        if output.is_empty() {
            None
        } else {
            Some(output)
        }
    }

    /// Apply syntax highlighting to a chunk.
    ///
    /// For code blocks, this wraps the highlighted content with visible fences
    /// to make the block boundaries clear in the terminal output.
    ///
    /// For prose, markdown formatting is applied using termimad when terminal
    /// output is enabled. This renders headers, bold, italic, inline code,
    /// and lists with appropriate ANSI styling.
    fn highlight_chunk(&self, chunk: &ParsedChunk) -> String {
        match &chunk.chunk_type {
            ChunkType::Prose => {
                if self.highlighting_enabled {
                    // Render markdown formatting with termimad
                    self.markdown_renderer.render_line(&chunk.content)
                } else {
                    // Plain text for non-terminal output
                    chunk.content.clone()
                }
            }
            ChunkType::Code { language } => {
                // Format the opening fence with language hint
                let opening_fence = match language {
                    Some(lang) if !lang.is_empty() => format!("```{}", lang),
                    _ => "```".to_string(),
                };

                // Highlight the code content (or leave plain if highlighting disabled)
                let highlighted_content = if self.highlighting_enabled {
                    let lang_ref = language.as_deref();
                    self.code_highlighter.highlight(&chunk.content, lang_ref)
                } else {
                    chunk.content.clone()
                };

                // Build the full block with fences
                format!("{}\n{}\n```", opening_fence, highlighted_content)
            }
            ChunkType::Diff => {
                // Format with visible diff fence
                let opening_fence = "```diff";
                let closing_fence = "```";

                // Highlight the diff content
                let highlighted_content = if self.highlighting_enabled {
                    highlight_with_basic_colors(&chunk.content)
                } else {
                    chunk.content.clone()
                };

                format!(
                    "{}\n{}\n{}",
                    opening_fence, highlighted_content, closing_fence
                )
            }
        }
    }

    /// Flush any pending chunks from the buffer.
    fn flush_pending_chunks(&mut self) -> Option<String> {
        let chunks = self.chunk_buffer.finish();
        if chunks.is_empty() {
            return None;
        }

        let mut output = String::new();
        for chunk in chunks {
            let highlighted = self.highlight_chunk(&chunk);
            output.push_str(&highlighted);
            output.push('\n');
            self.collected_chunks.push(chunk);
        }

        if output.is_empty() {
            None
        } else {
            Some(output)
        }
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
        // Flush remaining chunks
        let final_chunks = self.chunk_buffer.finish();
        self.collected_chunks.extend(final_chunks);

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
}

/// Result of extracting a key argument from a tool invocation.
///
/// Contains the argument value and metadata about whether it should be
/// displayed in full (e.g., file paths) or truncated (e.g., file content).
#[derive(Debug, Clone, PartialEq)]
pub struct KeyArgument {
    /// The argument value.
    pub value: String,
    /// Whether this is a file path that should be shown in full.
    pub is_path: bool,
}

/// Extract the most relevant argument from a tool invocation for display.
///
/// Different tools have different key arguments:
/// - Read/Edit/Write: file_path (shown in full)
/// - Glob: pattern (shown in full)
/// - Grep: pattern (shown in full)
/// - Bash: command (truncated)
/// - WebFetch: url (shown in full)
/// - Task: prompt (truncated)
fn extract_key_argument(tool_name: &str, input: &Value) -> Option<KeyArgument> {
    let obj = input.as_object()?;

    // Tool-specific key arguments with path indicators
    let (key, is_path) = match tool_name {
        "Read" | "Edit" | "Write" => ("file_path", true),
        "Glob" => ("pattern", true),
        "Grep" => ("pattern", true),
        "Bash" => ("command", false),
        "WebFetch" => ("url", true),
        "Task" => ("prompt", false),
        "NotebookEdit" => ("notebook_path", true),
        _ => {
            // For unknown tools, try common field names
            if obj.contains_key("file_path") {
                ("file_path", true)
            } else if obj.contains_key("path") {
                ("path", true)
            } else if obj.contains_key("pattern") {
                ("pattern", true)
            } else if obj.contains_key("command") {
                ("command", false)
            } else {
                // Return the first string value (truncated since we don't know what it is)
                for (_, v) in obj {
                    if let Some(s) = v.as_str() {
                        return Some(KeyArgument {
                            value: s.to_string(),
                            is_path: false,
                        });
                    }
                }
                return None;
            }
        }
    };

    obj.get(key).and_then(|v| v.as_str()).map(|s| KeyArgument {
        value: s.to_string(),
        is_path,
    })
}

/// Truncate a string to a maximum length, adding ellipsis if needed.
fn truncate_string(s: &str, max_len: usize) -> String {
    // First, replace newlines with spaces for cleaner display
    let single_line: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();

    // Then truncate if needed
    if single_line.len() <= max_len {
        single_line
    } else {
        let truncated: String = single_line.chars().take(max_len - 3).collect();
        format!("{}...", truncated)
    }
}

/// Truncate multiline content to a maximum number of lines.
///
/// Returns a tuple of (truncated content, was_truncated).
fn truncate_multiline(s: &str, max_lines: usize) -> (String, bool) {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= max_lines {
        (s.to_string(), false)
    } else {
        let truncated = lines[..max_lines].join("\n");
        (truncated, true)
    }
}

#[cfg(test)]
#[path = "stream_processor_tests.rs"]
mod tests;
