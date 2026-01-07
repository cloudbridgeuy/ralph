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
use crate::highlight::Highlighter;
use ralph_core::chunk::{
    split_lines_preserve_trailing, ChunkType, ParsedChunk, StreamingChunkBuffer,
};
use ralph_core::stream::{
    correlate_tool_interactions, extract_costs_from_events_or_default,
    extract_metadata_from_events_or_default, parse_stream_line, IterationCosts, IterationMetadata,
    ParsedLine, StreamEvent, ToolInteraction,
};
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
    /// Whether highlighting is enabled (terminal detection).
    highlighting_enabled: bool,
    /// Current message ID for accumulation.
    current_message_id: Option<String>,
    /// Chunks collected during streaming.
    collected_chunks: Vec<ParsedChunk>,
    /// Parse errors encountered.
    parse_errors: Vec<(String, String)>,
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
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::stream_processor::StreamProcessor;
    ///
    /// let processor = StreamProcessor::new();
    /// ```
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            text_buffer: String::new(),
            chunk_buffer: StreamingChunkBuffer::new(),
            code_highlighter: Highlighter::new(),
            diff_highlighter: DiffHighlighter::new(),
            highlighting_enabled: std::io::stdout().is_terminal(),
            current_message_id: None,
            collected_chunks: Vec::new(),
            parse_errors: Vec::new(),
        }
    }

    /// Create a processor with highlighting explicitly enabled/disabled.
    ///
    /// Useful for testing or when output will be displayed later.
    pub fn with_highlighting(enabled: bool) -> Self {
        Self {
            highlighting_enabled: enabled,
            ..Self::new()
        }
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
        let output = match &event {
            StreamEvent::Assistant(assistant_event) => {
                // Check if this is a new message
                let new_message_id = assistant_event.message.id.clone();
                if new_message_id != self.current_message_id && self.current_message_id.is_some() {
                    // New message - flush any pending content
                    let _ = self.flush_pending_chunks();
                }
                self.current_message_id = new_message_id;

                // Extract text from this event
                let text = assistant_event.extract_text();
                if !text.is_empty() {
                    self.text_buffer.push_str(&text);
                    // Process through chunk buffer and output
                    self.process_text_for_output(&text)
                } else {
                    None
                }
            }
            StreamEvent::System(_) | StreamEvent::User(_) | StreamEvent::Result(_) => {
                // Non-assistant events don't produce output
                None
            }
        };

        // Store event for post-processing
        self.events.push(event);

        output
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
    fn highlight_chunk(&self, chunk: &ParsedChunk) -> String {
        if !self.highlighting_enabled {
            return chunk.content.clone();
        }

        match &chunk.chunk_type {
            ChunkType::Prose => chunk.content.clone(),
            ChunkType::Code { language } => {
                let lang_ref = language.as_deref();
                self.code_highlighter.highlight(&chunk.content, lang_ref)
            }
            ChunkType::Diff => {
                // Use basic colors always for inline output
                highlight_with_basic_colors(&chunk.content)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_processor_new() {
        let processor = StreamProcessor::new();
        assert!(processor.raw_text().is_empty());
        assert!(processor.parse_errors().is_empty());
    }

    #[test]
    fn test_stream_processor_with_highlighting() {
        let processor = StreamProcessor::with_highlighting(true);
        assert!(processor.is_highlighting_enabled());

        let processor = StreamProcessor::with_highlighting(false);
        assert!(!processor.is_highlighting_enabled());
    }

    #[test]
    fn test_process_empty_line() {
        let mut processor = StreamProcessor::new();
        let output = processor.process_line("");
        assert!(output.is_none());

        let output = processor.process_line("   ");
        assert!(output.is_none());
    }

    #[test]
    fn test_process_malformed_json() {
        let mut processor = StreamProcessor::new();
        let output = processor.process_line("not json");
        assert!(output.is_none());
        assert_eq!(processor.parse_errors().len(), 1);
    }

    #[test]
    fn test_process_system_event() {
        let mut processor = StreamProcessor::new();
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude"}"#;
        let _output = processor.process_line(line);
        // System events don't produce output - processor stores them for metadata extraction
    }

    #[test]
    fn test_process_assistant_text_event() {
        let mut processor = StreamProcessor::with_highlighting(false);
        let line = r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"Hello, world!"}]}}"#;
        let _output = processor.process_line(line);

        // Text should be captured
        assert!(processor.raw_text().contains("Hello, world!"));
    }

    #[test]
    fn test_process_result_event() {
        let mut processor = StreamProcessor::new();
        let line = r#"{"type":"result","duration_ms":1000,"total_cost_usd":0.05,"usage":{"input_tokens":100,"output_tokens":50}}"#;
        let output = processor.process_line(line);
        assert!(output.is_none()); // Result events don't produce output
    }

    #[test]
    fn test_finish_extracts_metadata() {
        let mut processor = StreamProcessor::new();
        processor.process_line(
            r#"{"type":"system","subtype":"init","session_id":"test-session","model":"claude-3"}"#,
        );
        processor.process_line(
            r#"{"type":"result","duration_ms":5000,"total_cost_usd":0.10,"usage":{"input_tokens":200,"output_tokens":100}}"#,
        );

        let result = processor.finish();
        assert_eq!(result.metadata.session_id.as_deref(), Some("test-session"));
        assert_eq!(result.metadata.model.as_deref(), Some("claude-3"));
        assert_eq!(result.costs.cost_usd, Some(0.10));
        assert_eq!(result.costs.duration_ms, Some(5000));
    }

    #[test]
    fn test_finish_returns_accumulated_text() {
        let mut processor = StreamProcessor::with_highlighting(false);
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"First "}]}}"#,
        );
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Second"}]}}"#,
        );

        let result = processor.finish();
        assert!(result.raw_text.contains("First"));
        assert!(result.raw_text.contains("Second"));
    }

    #[test]
    fn test_tool_interaction_correlation() {
        let mut processor = StreamProcessor::new();

        // Tool invocation
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"/test"}}]}}"#,
        );

        // Tool result
        processor.process_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":"file contents"}]}}"#,
        );

        let result = processor.finish();
        assert_eq!(result.tool_interactions.len(), 1);
        assert_eq!(result.tool_interactions[0].name, "Read");
        assert!(result.tool_interactions[0].result.is_some());
    }

    #[test]
    fn test_code_block_detection() {
        let mut processor = StreamProcessor::with_highlighting(false);

        // Send text with a code block
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Here is code:"}]}}"#,
        );
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"\n```rust\nfn main() {}\n```"}]}}"#,
        );

        let result = processor.finish();
        assert!(!result.chunks.is_empty());
        // Should have captured the code block
        let has_code = result
            .chunks
            .iter()
            .any(|c| matches!(c.chunk_type, ChunkType::Code { .. }));
        assert!(has_code, "Should have detected code block");
    }

    #[test]
    fn test_diff_block_detection() {
        let mut processor = StreamProcessor::with_highlighting(false);

        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```diff\n+added\n-removed\n```"}]}}"#,
        );

        let result = processor.finish();
        let has_diff = result
            .chunks
            .iter()
            .any(|c| matches!(c.chunk_type, ChunkType::Diff));
        assert!(has_diff, "Should have detected diff block");
    }

    #[test]
    fn test_multiple_messages() {
        let mut processor = StreamProcessor::with_highlighting(false);

        // First message
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"msg-1","content":[{"type":"text","text":"First message"}]}}"#,
        );

        // Second message (different ID)
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"msg-2","content":[{"type":"text","text":"Second message"}]}}"#,
        );

        let result = processor.finish();
        assert!(result.raw_text.contains("First message"));
        assert!(result.raw_text.contains("Second message"));
    }

    #[test]
    fn test_empty_finish() {
        let processor = StreamProcessor::new();
        let result = processor.finish();
        assert!(result.chunks.is_empty());
        assert!(result.raw_text.is_empty());
        assert!(result.tool_interactions.is_empty());
    }

    // ==========================================================================
    // Whitespace preservation tests
    // ==========================================================================

    #[test]
    fn test_whitespace_blank_lines_preserved_between_paragraphs() {
        let mut processor = StreamProcessor::with_highlighting(false);

        // Simulate: "Paragraph 1.\n\nParagraph 2."
        let output1 = processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Paragraph 1.\n\nParagraph 2."}]}}"#,
        );

        let result = processor.finish();

        // Should have three chunks: Paragraph 1, blank line, Paragraph 2
        assert_eq!(result.chunks.len(), 3);
        assert_eq!(result.chunks[0].content, "Paragraph 1.");
        assert_eq!(result.chunks[1].content, ""); // blank line preserved
        assert_eq!(result.chunks[2].content, "Paragraph 2.");

        // raw_text should preserve the original
        assert_eq!(result.raw_text, "Paragraph 1.\n\nParagraph 2.");

        // Output should have correct newlines
        if let Some(out) = output1 {
            // Each chunk gets a newline, so: "Paragraph 1.\n" + "\n" + "Paragraph 2.\n"
            assert_eq!(out, "Paragraph 1.\n\nParagraph 2.\n");
        }
    }

    #[test]
    fn test_whitespace_multiple_blank_lines_preserved() {
        let mut processor = StreamProcessor::with_highlighting(false);

        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Text\n\n\nMore text"}]}}"#,
        );

        let result = processor.finish();

        // Should have: Text, blank, blank, More text
        assert_eq!(result.chunks.len(), 4);
        assert_eq!(result.chunks[0].content, "Text");
        assert_eq!(result.chunks[1].content, "");
        assert_eq!(result.chunks[2].content, "");
        assert_eq!(result.chunks[3].content, "More text");
    }

    #[test]
    fn test_whitespace_code_block_content_preserved() {
        let mut processor = StreamProcessor::with_highlighting(false);

        // Code with internal blank line
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```rust\nfn a() {}\n\nfn b() {}\n```"}]}}"#,
        );

        let result = processor.finish();

        // Find the code chunk
        let code_chunk = result
            .chunks
            .iter()
            .find(|c| matches!(c.chunk_type, ChunkType::Code { .. }))
            .expect("Should have code chunk");

        // Internal blank line should be preserved
        assert_eq!(code_chunk.content, "fn a() {}\n\nfn b() {}");
    }

    #[test]
    fn test_whitespace_indentation_preserved_in_code() {
        let mut processor = StreamProcessor::with_highlighting(false);

        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```python\ndef foo():\n    x = 1\n        nested = 2\n```"}]}}"#,
        );

        let result = processor.finish();

        let code_chunk = result
            .chunks
            .iter()
            .find(|c| matches!(c.chunk_type, ChunkType::Code { .. }))
            .expect("Should have code chunk");

        // Indentation preserved exactly
        assert_eq!(
            code_chunk.content,
            "def foo():\n    x = 1\n        nested = 2"
        );
    }

    #[test]
    fn test_whitespace_trailing_newline_in_text() {
        let mut processor = StreamProcessor::with_highlighting(false);

        // Text with trailing newline
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Line 1\nLine 2\n"}]}}"#,
        );

        let result = processor.finish();

        // Should preserve trailing newline as empty chunk
        assert_eq!(result.chunks.len(), 3);
        assert_eq!(result.chunks[0].content, "Line 1");
        assert_eq!(result.chunks[1].content, "Line 2");
        assert_eq!(result.chunks[2].content, ""); // trailing newline
    }

    #[test]
    fn test_whitespace_leading_spaces_preserved() {
        let mut processor = StreamProcessor::with_highlighting(false);

        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"    indented line"}]}}"#,
        );

        let result = processor.finish();

        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].content, "    indented line");
    }

    #[test]
    fn test_whitespace_list_indentation_preserved() {
        let mut processor = StreamProcessor::with_highlighting(false);

        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"- Item 1\n  - Nested item\n    - Deeply nested"}]}}"#,
        );

        let result = processor.finish();

        assert_eq!(result.chunks.len(), 3);
        assert_eq!(result.chunks[0].content, "- Item 1");
        assert_eq!(result.chunks[1].content, "  - Nested item");
        assert_eq!(result.chunks[2].content, "    - Deeply nested");
    }

    #[test]
    fn test_whitespace_blank_line_before_code_block() {
        let mut processor = StreamProcessor::with_highlighting(false);

        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Here's code:\n\n```rust\nfn main() {}\n```"}]}}"#,
        );

        let result = processor.finish();

        // Should have: prose ("Here's code:"), blank line, code block
        assert_eq!(result.chunks.len(), 3);
        assert_eq!(result.chunks[0].content, "Here's code:");
        assert_eq!(result.chunks[1].content, ""); // blank line before code
        assert!(matches!(
            result.chunks[2].chunk_type,
            ChunkType::Code { .. }
        ));
    }

    #[test]
    fn test_whitespace_blank_line_after_code_block() {
        let mut processor = StreamProcessor::with_highlighting(false);

        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```rust\nfn main() {}\n```\n\nDone."}]}}"#,
        );

        let result = processor.finish();

        // Should have: code block, blank line, prose ("Done.")
        assert_eq!(result.chunks.len(), 3);
        assert!(matches!(
            result.chunks[0].chunk_type,
            ChunkType::Code { .. }
        ));
        assert_eq!(result.chunks[1].content, ""); // blank line after code
        assert_eq!(result.chunks[2].content, "Done.");
    }

    #[test]
    fn test_whitespace_raw_text_matches_original() {
        let mut processor = StreamProcessor::with_highlighting(false);

        let original = "Hello\n\nWorld\n\n```rust\ncode\n```\n\nDone";
        processor.process_line(&format!(
            r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"text","text":"{}"}}]}}}}"#,
            original.replace('\n', "\\n")
        ));

        let result = processor.finish();

        // raw_text should match original exactly
        assert_eq!(result.raw_text, original);
    }

    #[test]
    fn test_whitespace_across_multiple_events() {
        let mut processor = StreamProcessor::with_highlighting(false);

        // First event ends mid-paragraph
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Hello "}]}}"#,
        );

        // Second event continues
        processor.process_line(
            r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"World\n\nNext paragraph"}]}}"#,
        );

        let result = processor.finish();

        // raw_text should be "Hello World\n\nNext paragraph"
        assert_eq!(result.raw_text, "Hello World\n\nNext paragraph");
    }
}
