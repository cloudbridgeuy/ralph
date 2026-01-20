//! Event processing and chunk rendering for the stream processor.
//!
//! This module implements the core event processing loop including
//! JSON line parsing, event routing, and text/chunk output generation.

use std::fs;

use crate::diff_highlight::highlight_with_basic_colors;
use ralph_core::chunk::{split_lines_preserve_trailing, ChunkType, ParsedChunk};
use ralph_core::stream::{parse_stream_line, ParsedLine, StreamEvent, ToolInvocation};

use super::block_builders::{
    build_bash_result_block, build_default_result_block, build_edit_before_after_block,
    build_edit_diff_block, build_glob_result_block, build_grep_result_block,
    build_notebook_edit_block, build_read_result_block, build_todowrite_result_block,
    build_tool_invocation_block, build_write_result_block,
};
use super::output_block::OutputBlock;
use super::processor::StreamProcessor;
use super::tool_display;
use super::tool_results;
use super::types::{EditSnapshot, NotebookSnapshot, WriteSnapshot};
use super::utils::{count_non_empty_lines, is_content_truncated};

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
                        // Look up the original invocation to get the tool name
                        let invocation = result
                            .tool_use_id
                            .as_ref()
                            .and_then(|id| self.pending_invocations.remove(id));

                        // Remove any pending Edit snapshot (cleanup)
                        let edit_snapshot = result
                            .tool_use_id
                            .as_ref()
                            .and_then(|id| self.pending_edit_snapshots.remove(id));

                        // Remove any pending Write snapshot (cleanup)
                        let write_snapshot = result
                            .tool_use_id
                            .as_ref()
                            .and_then(|id| self.pending_write_snapshots.remove(id));

                        // Remove any pending NotebookEdit snapshot (cleanup)
                        let notebook_snapshot = result
                            .tool_use_id
                            .as_ref()
                            .and_then(|id| self.pending_notebook_snapshots.remove(id));

                        // Capture invocation info for block building before the match consumes it
                        let invocation_name = invocation.as_ref().map(|i| i.name.clone());

                        // Determine formatting approach and build output block based on tool type and snapshots
                        let (formatted, block) = match &invocation {
                            Some(inv) if inv.name == "Edit" && !result.is_error => {
                                // Edit tool - check for diff content or use snapshot
                                if let Some(ref snap) = edit_snapshot {
                                    let has_diff_content = result
                                        .content
                                        .as_ref()
                                        .map(|c| ralph_core::chunk::is_unfenced_diff(c))
                                        .unwrap_or(false);

                                    if has_diff_content {
                                        // Result contains diff - use diff formatting and block
                                        let fmt = tool_results::format_tool_result_with_context(
                                            self,
                                            result,
                                            Some(inv.clone()),
                                        );
                                        let blk = build_edit_diff_block(
                                            &snap.file_path,
                                            result.content.as_deref().unwrap_or(""),
                                        );
                                        (fmt, blk)
                                    } else {
                                        // No diff in result - generate from snapshot
                                        let fmt = tool_results::format_edit_result_with_snapshot(
                                            self,
                                            snap.clone(),
                                        );
                                        let blk = build_edit_before_after_block(snap);
                                        (fmt, blk)
                                    }
                                } else {
                                    let fmt = tool_results::format_tool_result_with_context(
                                        self,
                                        result,
                                        invocation.clone(),
                                    );
                                    let blk = build_default_result_block(
                                        &inv.name,
                                        result.content.as_deref(),
                                        result.is_error,
                                    );
                                    (fmt, blk)
                                }
                            }
                            Some(inv) if inv.name == "Write" && !result.is_error => {
                                // Write tool - generate diff from snapshot
                                if let Some(ref snap) = write_snapshot {
                                    let fmt = tool_results::format_write_result_with_snapshot(
                                        self,
                                        snap.clone(),
                                    );
                                    // Read the new file content to determine which variant to use
                                    let new_content = std::fs::read_to_string(&snap.file_path).ok();
                                    let blk =
                                        build_write_result_block(snap, new_content.as_deref());
                                    (fmt, blk)
                                } else {
                                    let fmt = tool_results::format_tool_result_with_context(
                                        self,
                                        result,
                                        invocation.clone(),
                                    );
                                    let blk = build_default_result_block(
                                        &inv.name,
                                        result.content.as_deref(),
                                        result.is_error,
                                    );
                                    (fmt, blk)
                                }
                            }
                            Some(inv) if inv.name == "NotebookEdit" && !result.is_error => {
                                // NotebookEdit tool - generate diff from snapshot
                                if let Some(ref snap) = notebook_snapshot {
                                    let fmt = tool_results::format_notebook_result_with_snapshot(
                                        self,
                                        snap.clone(),
                                    );
                                    // Extract new_source from invocation input
                                    let new_source = inv
                                        .input
                                        .get("new_source")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let blk = build_notebook_edit_block(snap, new_source);
                                    (fmt, blk)
                                } else {
                                    let fmt = tool_results::format_tool_result_with_context(
                                        self,
                                        result,
                                        invocation.clone(),
                                    );
                                    let blk = build_default_result_block(
                                        &inv.name,
                                        result.content.as_deref(),
                                        result.is_error,
                                    );
                                    (fmt, blk)
                                }
                            }
                            Some(inv) if inv.name == "Bash" => {
                                // Bash tool - use specialized block
                                let fmt = tool_results::format_tool_result_with_context(
                                    self,
                                    result,
                                    invocation.clone(),
                                );
                                let blk = build_bash_result_block(
                                    result.content.as_deref(),
                                    result.is_error,
                                );
                                (fmt, blk)
                            }
                            Some(inv) if inv.name == "Read" && !result.is_error => {
                                // Read tool - extract metadata for specialized block
                                let fmt = tool_results::format_tool_result_with_context(
                                    self,
                                    result,
                                    invocation.clone(),
                                );
                                let content = result.content.as_deref().unwrap_or("");
                                let line_count = content.lines().count();
                                let truncated = is_content_truncated(content);
                                let file_path = inv
                                    .input
                                    .get("file_path")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let blk = build_read_result_block(
                                    file_path, content, line_count, truncated,
                                );
                                (fmt, blk)
                            }
                            Some(inv) if inv.name == "Grep" && !result.is_error => {
                                // Grep tool - extract metadata for specialized block
                                let fmt = tool_results::format_tool_result_with_context(
                                    self,
                                    result,
                                    invocation.clone(),
                                );
                                let content = result.content.as_deref().unwrap_or("");
                                let match_count = count_non_empty_lines(content);
                                let output_mode = inv
                                    .input
                                    .get("output_mode")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("files_with_matches");
                                let blk =
                                    build_grep_result_block(match_count, output_mode, content);
                                (fmt, blk)
                            }
                            Some(inv) if inv.name == "Glob" && !result.is_error => {
                                // Glob tool - extract metadata for specialized block
                                let fmt = tool_results::format_tool_result_with_context(
                                    self,
                                    result,
                                    invocation.clone(),
                                );
                                let content = result.content.as_deref().unwrap_or("");
                                let file_count = count_non_empty_lines(content);
                                let truncated = is_content_truncated(content);
                                let blk = build_glob_result_block(file_count, content, truncated);
                                (fmt, blk)
                            }
                            Some(inv) if inv.name == "TodoWrite" => {
                                // TodoWrite tool - use specialized block (message is from result)
                                let fmt = tool_results::format_tool_result_with_context(
                                    self,
                                    result,
                                    invocation.clone(),
                                );
                                let blk = build_todowrite_result_block(result.content.as_deref());
                                (fmt, blk)
                            }
                            _ => {
                                // Other tools or errors - use regular formatting
                                let fmt = tool_results::format_tool_result_with_context(
                                    self,
                                    result,
                                    invocation.clone(),
                                );
                                let tool_name = invocation_name.as_deref().unwrap_or("Unknown");
                                let blk = build_default_result_block(
                                    tool_name,
                                    result.content.as_deref(),
                                    result.is_error,
                                );
                                (fmt, blk)
                            }
                        };
                        output_parts.push(formatted);

                        // Accumulate tool result block for replay
                        self.output_blocks.push(block);
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
