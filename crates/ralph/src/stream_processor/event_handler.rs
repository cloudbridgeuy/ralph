//! Event processing and chunk rendering for the stream processor.
//!
//! This module implements the core event processing loop including
//! JSON line parsing, event routing, and text/chunk output generation.

use ralph_core::chat_send_it::is_step_result_tool;
use ralph_core::chunk::{split_lines_preserve_trailing, ChunkType, ParsedChunk};
use ralph_core::directive::split_text_around_directives;
use ralph_core::stream::{parse_stream_line, ParsedLine, StreamEvent, ToolInvocation};

use super::disk_files::DiskFiles;
use super::processor::StreamProcessor;
use super::render_block::render_block;
use ralph_core::stream::blocks::{
    build_result_block, build_tool_invocation_block, capture_edit_snapshot,
    capture_notebook_snapshot, capture_write_snapshot, Files, OutputBlock, Snapshot,
};

fn capture_snapshot(invocation: &ToolInvocation, files: &dyn Files) -> Option<Snapshot> {
    match invocation.name.as_str() {
        "Edit" => capture_edit_snapshot(invocation, files).map(Snapshot::Edit),
        "Write" => capture_write_snapshot(invocation, files).map(Snapshot::Write),
        "NotebookEdit" => capture_notebook_snapshot(invocation, files).map(Snapshot::Notebook),
        _ => None,
    }
}

impl StreamProcessor {
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
                    // (but only if we've already emitted some output, including
                    // content just flushed from the buffer above)
                    if self.has_emitted_output || !output_parts.is_empty() {
                        output_parts.push("\n".to_string());
                        // Accumulate separator block
                        self.output_blocks.push(OutputBlock::separator());
                    }
                    self.response_count += 1;
                } else if self.current_message_id.is_none() {
                    // First response
                    self.response_count = 1;
                }
                self.current_message_id = new_message_id;

                // Display tool invocations if enabled, and track them for result formatting
                let tool_invocations = assistant_event.extract_tool_invocations();

                // Before processing tool invocations, flush any buffered prose
                if !tool_invocations.is_empty() {
                    if let Some(flushed) = self.flush_pending_chunks() {
                        output_parts.push(flushed);
                    }
                }

                for invocation in &tool_invocations {
                    if !self.show_tool_invocations {
                        continue;
                    }
                    self.pending_invocations
                        .insert(invocation.id.clone(), invocation.clone());

                    if let Some(snapshot) = capture_snapshot(invocation, &DiskFiles) {
                        self.pending_snapshots
                            .insert(invocation.id.clone(), snapshot);
                    }

                    if is_step_result_tool(&invocation.name) {
                        continue;
                    }
                    let block = build_tool_invocation_block(invocation, &self.verbose_tools_config);
                    output_parts.push(self.emit(block));
                }

                // Extract text from this event
                let text = assistant_event.extract_text();
                if !text.is_empty() {
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
                        let tool_use_id = result.tool_use_id.as_ref();
                        let invocation =
                            tool_use_id.and_then(|id| self.pending_invocations.remove(id));
                        let snapshot = tool_use_id.and_then(|id| self.pending_snapshots.remove(id));

                        if !result.is_error
                            && invocation
                                .as_ref()
                                .is_some_and(|inv| is_step_result_tool(&inv.name))
                        {
                            continue;
                        }

                        let block = build_result_block(
                            &self.verbose_tools_config,
                            result,
                            invocation.as_ref(),
                            snapshot,
                            &DiskFiles,
                        );
                        output_parts.push(self.emit(block));
                    }
                }
            }
            StreamEvent::System(_) | StreamEvent::Result(_) => {
                // System and result events don't produce visible output
            }
        }

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

    fn emit(&mut self, block: OutputBlock) -> String {
        let markdown = self.highlighting_enabled.then_some(&self.markdown);
        let rendered = render_block(&self.render_context(), &block, markdown);
        self.output_blocks.push(block);
        rendered
    }

    fn emit_chunks(&mut self, chunks: Vec<ParsedChunk>) -> String {
        let mut output = String::new();
        for chunk in chunks {
            let sub_chunks = match &chunk.chunk_type {
                ChunkType::Prose => split_text_around_directives(&chunk.content),
                _ => vec![chunk],
            };
            for sub_chunk in sub_chunks {
                self.collected_chunks.push(sub_chunk.clone());
                output.push_str(&self.emit(OutputBlock::text(sub_chunk)));
            }
        }
        output
    }

    pub(super) fn process_text_for_output(&mut self, text: &str) -> Option<String> {
        let mut output = String::new();
        for line in split_lines_preserve_trailing(text) {
            let chunks = self.chunk_buffer.process_line(line);
            output.push_str(&self.emit_chunks(chunks));
        }
        Some(output).filter(|o| !o.is_empty())
    }

    pub(super) fn flush_pending_chunks(&mut self) -> Option<String> {
        let chunks = self.chunk_buffer.finish();
        Some(self.emit_chunks(chunks)).filter(|o| !o.is_empty())
    }
}
