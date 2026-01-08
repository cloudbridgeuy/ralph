//! Event processing and chunk rendering for the stream processor.
//!
//! This module implements the core event processing loop including
//! JSON line parsing, event routing, and text/chunk output generation.

use std::fs;

use crate::diff_highlight::highlight_with_basic_colors;
use ralph_core::chunk::{split_lines_preserve_trailing, ChunkType, ParsedChunk};
use ralph_core::stream::{parse_stream_line, ParsedLine, StreamEvent, ToolInvocation};

use super::processor::StreamProcessor;
use super::tool_display;
use super::tool_results;
use super::types::EditSnapshot;

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

                    // Capture file content before Edit tool executes
                    if invocation.name == "Edit" {
                        if let Some(snapshot) = capture_edit_snapshot(invocation) {
                            self.pending_edit_snapshots
                                .insert(invocation.id.clone(), snapshot);
                        }
                    }

                    if self.show_tool_invocations {
                        let formatted = tool_display::format_tool_invocation(self, invocation);
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

                        // Remove any pending Edit snapshot (cleanup)
                        let snapshot = result
                            .tool_use_id
                            .as_ref()
                            .and_then(|id| self.pending_edit_snapshots.remove(id));

                        // Determine formatting approach for Edit tool results:
                        // 1. If result already contains diff content, use that (existing behavior)
                        // 2. Otherwise, try snapshot-based diff generation (new behavior)
                        let formatted = if let (Some(ref inv), Some(snap)) = (&invocation, snapshot)
                        {
                            if inv.name == "Edit" && !result.is_error {
                                // Check if result already contains diff content
                                let has_diff_content = result
                                    .content
                                    .as_ref()
                                    .map(|c| ralph_core::chunk::is_unfenced_diff(c))
                                    .unwrap_or(false);

                                if has_diff_content {
                                    // Result contains diff - use existing formatting
                                    tool_results::format_tool_result_with_context(
                                        self,
                                        result,
                                        Some(inv.clone()),
                                    )
                                } else {
                                    // No diff in result - generate from snapshot
                                    tool_results::format_edit_result_with_snapshot(self, snap)
                                }
                            } else {
                                // Error or non-Edit tool
                                tool_results::format_tool_result_with_context(
                                    self, result, invocation,
                                )
                            }
                        } else {
                            // No snapshot or invocation - use regular formatting
                            tool_results::format_tool_result_with_context(self, result, invocation)
                        };
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

    /// Process text through the chunk buffer and generate highlighted output.
    pub(super) fn process_text_for_output(&mut self, text: &str) -> Option<String> {
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
    pub(super) fn highlight_chunk(&self, chunk: &ParsedChunk) -> String {
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
    pub(super) fn flush_pending_chunks(&mut self) -> Option<String> {
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
}

/// Capture the current content of a file before an Edit tool modifies it.
///
/// This is a pure function that reads the file and returns a snapshot.
/// Returns None if the file path cannot be extracted from the invocation input.
fn capture_edit_snapshot(invocation: &ToolInvocation) -> Option<EditSnapshot> {
    // Extract file path from invocation input
    let file_path = invocation.input.get("file_path").and_then(|v| v.as_str())?;

    // Read current content - Ok(content) if file exists, None if it doesn't
    let content = fs::read_to_string(file_path).ok();

    Some(EditSnapshot {
        file_path: file_path.to_string(),
        content,
    })
}
