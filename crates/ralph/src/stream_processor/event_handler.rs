//! Event processing and chunk rendering for the stream processor.
//!
//! This module implements the core event processing loop including
//! JSON line parsing, event routing, and text/chunk output generation.

use std::fs;

use crate::render::{render_text_block, RenderContext};
use ralph_core::chunk::{split_lines_preserve_trailing, ChunkType, ParsedChunk};
use ralph_core::stream::{parse_stream_line, ParsedLine, StreamEvent, ToolInvocation};

use super::block_builders::build_tool_invocation_block;
use super::output_block::OutputBlock;
use super::processor::StreamProcessor;
use super::result_handlers::{
    handle_bash_result, handle_default_result, handle_edit_result, handle_glob_result,
    handle_grep_result, handle_notebook_result, handle_read_result, handle_todowrite_result,
    handle_write_result,
};
use super::tool_display;
use super::types::{EditSnapshot, NotebookSnapshot, WriteSnapshot};

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
                        // Accumulate separator block for replay
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

                    // Capture file content before Write tool executes
                    if invocation.name == "Write" {
                        if let Some(snapshot) = capture_write_snapshot(invocation) {
                            self.pending_write_snapshots
                                .insert(invocation.id.clone(), snapshot);
                        }
                    }

                    // Capture cell content before NotebookEdit tool executes
                    if invocation.name == "NotebookEdit" {
                        if let Some(snapshot) = capture_notebook_snapshot(invocation) {
                            self.pending_notebook_snapshots
                                .insert(invocation.id.clone(), snapshot);
                        }
                    }

                    if self.show_tool_invocations {
                        let formatted = tool_display::format_tool_invocation(self, invocation);
                        output_parts.push(formatted);
                        // Accumulate tool invocation block for replay
                        let block = build_tool_invocation_block(invocation);
                        self.output_blocks.push(block);
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
                        // Extract tool_use_id once for looking up pending data
                        let tool_use_id = result.tool_use_id.as_ref();
                        let invocation =
                            tool_use_id.and_then(|id| self.pending_invocations.remove(id));
                        let edit_snapshot =
                            tool_use_id.and_then(|id| self.pending_edit_snapshots.remove(id));
                        let write_snapshot =
                            tool_use_id.and_then(|id| self.pending_write_snapshots.remove(id));
                        let notebook_snapshot =
                            tool_use_id.and_then(|id| self.pending_notebook_snapshots.remove(id));

                        // Route to dedicated handlers based on tool type.
                        // Bash/TodoWrite handle both success and error; file-modifying tools
                        // fall through to default on error since their custom handlers expect success.
                        let output = if let Some(inv) = &invocation {
                            match (inv.name.as_str(), result.is_error) {
                                ("Edit", false) => {
                                    handle_edit_result(self, result, inv, edit_snapshot.as_ref())
                                }
                                ("Write", false) => {
                                    handle_write_result(self, result, inv, write_snapshot.as_ref())
                                }
                                ("NotebookEdit", false) => handle_notebook_result(
                                    self,
                                    result,
                                    inv,
                                    notebook_snapshot.as_ref(),
                                ),
                                ("Bash", _) => handle_bash_result(self, result, Some(inv)),
                                ("Read", false) => handle_read_result(self, result, inv),
                                ("Grep", false) => handle_grep_result(self, result, inv),
                                ("Glob", false) => handle_glob_result(self, result, inv),
                                ("TodoWrite", _) => {
                                    handle_todowrite_result(self, result, Some(inv))
                                }
                                _ => handle_default_result(self, result, Some(inv)),
                            }
                        } else {
                            handle_default_result(self, result, None)
                        };
                        output_parts.push(output.formatted);
                        self.output_blocks.push(output.block);
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

                // Accumulate text block for replay
                self.output_blocks.push(OutputBlock::text(chunk.clone()));

                // Generate highlighted output
                let highlighted = self.highlight_chunk(&chunk);
                output.push_str(&highlighted);

                // Code/diff blocks need explicit trailing newline;
                // prose rendered by term_text() manages its own newlines
                match &chunk.chunk_type {
                    ChunkType::Code { .. } | ChunkType::Diff => output.push('\n'),
                    ChunkType::Prose => {}
                }
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
    /// Delegates to the shared `render_text_block()` function for all chunk types,
    /// ensuring consistency between streaming and replay rendering paths.
    pub(super) fn highlight_chunk(&self, chunk: &ParsedChunk) -> String {
        let ctx = self.create_render_context();
        let markdown_skin = if self.highlighting_enabled {
            Some(&self.markdown_skin)
        } else {
            None
        };
        render_text_block(&ctx, chunk, markdown_skin)
    }

    /// Create a render context for use with shared rendering functions.
    fn create_render_context(&self) -> RenderContext<'_> {
        RenderContext::new(&self.code_highlighter, self.highlighting_enabled)
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

            // Code/diff blocks need explicit trailing newline;
            // prose rendered by term_text() manages its own newlines
            match &chunk.chunk_type {
                ChunkType::Code { .. } | ChunkType::Diff => output.push('\n'),
                ChunkType::Prose => {}
            }

            // Accumulate text block for replay
            self.output_blocks.push(OutputBlock::text(chunk.clone()));
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
///
/// Extracts both the file content (for unified diff fallback) and
/// old_string/new_string from the tool input (for before/after display).
fn capture_edit_snapshot(invocation: &ToolInvocation) -> Option<EditSnapshot> {
    // Extract file path from invocation input
    let file_path = invocation.input.get("file_path").and_then(|v| v.as_str())?;

    // Read current content - Ok(content) if file exists, None if it doesn't
    let content = fs::read_to_string(file_path).ok();

    // Extract old_string and new_string from Edit tool input
    let old_string = invocation
        .input
        .get("old_string")
        .and_then(|v| v.as_str())
        .map(String::from);
    let new_string = invocation
        .input
        .get("new_string")
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(EditSnapshot {
        file_path: file_path.to_string(),
        content,
        old_string,
        new_string,
    })
}

/// Capture the current content of a file before a Write tool creates/overwrites it.
///
/// This is a pure function that reads the file (if it exists) and returns a snapshot.
/// Returns None if the file path cannot be extracted from the invocation input.
fn capture_write_snapshot(invocation: &ToolInvocation) -> Option<WriteSnapshot> {
    // Extract file path from invocation input
    let file_path = invocation.input.get("file_path").and_then(|v| v.as_str())?;

    // Check if file exists and read content
    let file_existed = std::path::Path::new(file_path).exists();
    let content = fs::read_to_string(file_path).ok();

    Some(WriteSnapshot {
        file_path: file_path.to_string(),
        content,
        file_existed,
    })
}

/// Capture the current content of a notebook cell before a NotebookEdit tool modifies it.
///
/// This function reads the notebook JSON and extracts the cell content before the edit.
/// Returns None if the notebook path cannot be extracted or the notebook cannot be parsed.
fn capture_notebook_snapshot(invocation: &ToolInvocation) -> Option<NotebookSnapshot> {
    // Extract notebook path from invocation input
    let notebook_path = invocation
        .input
        .get("notebook_path")
        .and_then(|v| v.as_str())?;

    // Extract cell identifier - prefer cell_id, fall back to cell_number
    let cell_identifier = invocation
        .input
        .get("cell_id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            invocation
                .input
                .get("cell_number")
                .and_then(|v| v.as_u64())
                .map(|n| n.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Extract edit mode (default to "replace")
    let edit_mode = invocation
        .input
        .get("edit_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("replace")
        .to_string();

    // Extract cell type if provided
    let cell_type = invocation
        .input
        .get("cell_type")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Try to read the notebook and extract cell content
    let content = read_notebook_cell_content(notebook_path, &cell_identifier);

    Some(NotebookSnapshot {
        notebook_path: notebook_path.to_string(),
        cell_identifier,
        content,
        edit_mode,
        cell_type,
    })
}

/// Read the content of a specific cell from a Jupyter notebook.
///
/// Returns the cell content as a single string (joining source lines),
/// or None if the notebook or cell cannot be read.
fn read_notebook_cell_content(notebook_path: &str, cell_identifier: &str) -> Option<String> {
    // Read and parse the notebook JSON
    let notebook_content = fs::read_to_string(notebook_path).ok()?;
    let notebook: serde_json::Value = serde_json::from_str(&notebook_content).ok()?;

    // Get the cells array
    let cells = notebook.get("cells")?.as_array()?;

    // Try to find the cell by identifier
    // First try to match by cell_id (string match)
    // Then try to match by index (if identifier is numeric)
    let cell = if let Ok(index) = cell_identifier.parse::<usize>() {
        // Numeric identifier - use as 0-based index
        cells.get(index)
    } else {
        // String identifier - try to match against cell id metadata
        cells.iter().find(|cell| {
            cell.get("id")
                .and_then(|id| id.as_str())
                .map(|id| id == cell_identifier)
                .unwrap_or(false)
        })
    }?;

    // Extract the source content
    // The "source" field can be either a string or an array of strings
    let source = cell.get("source")?;
    if let Some(s) = source.as_str() {
        Some(s.to_string())
    } else if let Some(arr) = source.as_array() {
        let lines: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        Some(lines.join(""))
    } else {
        None
    }
}
