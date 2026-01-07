//! Streaming chunk parser for incremental processing.

use super::fence::{is_fence_close, parse_fence_open};
use super::types::ParsedChunk;

/// Split text into lines while preserving trailing newlines.
///
/// Unlike `str::lines()`, this function preserves trailing empty lines.
/// For example:
/// - `"a\nb"` → `["a", "b"]`
/// - `"a\nb\n"` → `["a", "b", ""]` (preserves the trailing empty line)
/// - `"a\n\nb"` → `["a", "", "b"]` (preserves blank lines)
///
/// This is critical for whitespace preservation in streamed output,
/// ensuring the output matches the original LLM response byte-for-byte.
pub fn split_lines_preserve_trailing(text: &str) -> impl Iterator<Item = &str> {
    SplitLinesIter {
        text,
        finished: false,
        emit_trailing_empty: false,
    }
}

/// Iterator for splitting lines while preserving trailing newlines.
struct SplitLinesIter<'a> {
    text: &'a str,
    finished: bool,
    /// Whether we need to emit an empty string for a trailing newline.
    emit_trailing_empty: bool,
}

impl<'a> Iterator for SplitLinesIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        // Check if we need to emit an empty string for trailing newline
        if self.emit_trailing_empty {
            self.emit_trailing_empty = false;
            self.finished = true;
            return Some("");
        }

        if self.text.is_empty() {
            self.finished = true;
            return None;
        }

        // Find the next newline
        if let Some(pos) = self.text.find('\n') {
            // Check for CRLF
            let line_end = if pos > 0 && self.text.as_bytes().get(pos - 1) == Some(&b'\r') {
                pos - 1
            } else {
                pos
            };
            let line = &self.text[..line_end];
            self.text = &self.text[pos + 1..];

            // If we just consumed the last character (the newline), we need to
            // emit an empty string on the next iteration
            if self.text.is_empty() {
                self.emit_trailing_empty = true;
            }

            Some(line)
        } else {
            // No more newlines - return the rest
            let line = self.text;
            self.text = "";
            self.finished = true;
            Some(line)
        }
    }
}

/// Internal state for the streaming chunk buffer.
#[derive(Debug, Clone, PartialEq)]
enum BufferState {
    /// Processing prose content.
    /// When buffering is disabled (threshold = 0), lines emit eagerly.
    /// When buffering is enabled, lines accumulate until threshold is reached.
    Prose,
    /// Inside a fenced code block (buffer until closing fence).
    Code {
        language: Option<String>,
        is_diff: bool,
    },
}

/// Default number of prose lines to buffer before flushing.
/// Set to 0 for eager (line-by-line) streaming, or 3-5 for progressive buffering.
pub const DEFAULT_PROSE_BUFFER_THRESHOLD: usize = 0;

/// A streaming buffer for parsing LLM output into typed chunks.
///
/// This buffer processes text line by line, emitting complete chunks as soon
/// as their boundaries are detected. Code blocks and diff blocks are buffered
/// until their closing fence is seen.
///
/// # Prose Buffering
///
/// By default (threshold = 0), prose streams eagerly with each line emitted immediately.
/// When a prose buffer threshold is set (e.g., 3-5 lines), prose accumulates until
/// the threshold is reached, then emits as a single chunk. This provides a balance
/// between immediate feedback and reducing output noise.
///
/// # Design Principles
///
/// - **Code blocks buffer**: Not emitted until closing ``` is seen
/// - **Diff blocks buffer**: Treated the same as code blocks (```diff fence)
/// - **Prose streaming**: Configurable via threshold (0 = eager, N = buffer N lines)
/// - **Final flush**: Unterminated blocks are emitted on [`finish()`](Self::finish)
///
/// # Example
///
/// ```
/// use ralph_core::chunk::{StreamingChunkBuffer, ChunkType};
///
/// let mut buffer = StreamingChunkBuffer::new();
///
/// // Prose emitted eagerly (default threshold = 0)
/// let chunks = buffer.process_line("Hello, world!");
/// assert_eq!(chunks.len(), 1);
/// assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
///
/// // Code block buffered until closed
/// assert!(buffer.process_line("```rust").is_empty());
/// assert!(buffer.process_line("fn main() {}").is_empty());
///
/// let chunks = buffer.process_line("```");
/// assert_eq!(chunks.len(), 1);
/// assert!(matches!(chunks[0].chunk_type, ChunkType::Code { .. }));
/// ```
///
/// # Example with Prose Buffering
///
/// ```
/// use ralph_core::chunk::{StreamingChunkBuffer, ChunkType};
///
/// let mut buffer = StreamingChunkBuffer::with_prose_threshold(3);
///
/// // Lines buffer until threshold is reached
/// assert!(buffer.process_line("Line 1").is_empty());
/// assert!(buffer.process_line("Line 2").is_empty());
///
/// // Third line triggers flush
/// let chunks = buffer.process_line("Line 3");
/// assert_eq!(chunks.len(), 1);
/// assert_eq!(chunks[0].content, "Line 1\nLine 2\nLine 3");
/// ```
#[derive(Debug, Clone)]
pub struct StreamingChunkBuffer {
    /// Current parser state.
    state: BufferState,
    /// Accumulated content for the current chunk (code blocks or buffered prose).
    buffer: String,
    /// Count of emitted chunks (for debugging).
    emitted_count: usize,
    /// Number of prose lines to buffer before flushing (0 = emit each line immediately).
    prose_buffer_threshold: usize,
    /// Count of buffered prose lines (for threshold tracking).
    buffered_prose_lines: usize,
}

impl Default for StreamingChunkBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingChunkBuffer {
    /// Create a new empty streaming buffer with eager prose streaming (no buffering).
    ///
    /// This is equivalent to `StreamingChunkBuffer::with_prose_threshold(0)`.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::chunk::StreamingChunkBuffer;
    ///
    /// let buffer = StreamingChunkBuffer::new();
    /// assert!(buffer.is_empty());
    /// ```
    pub fn new() -> Self {
        Self::with_prose_threshold(DEFAULT_PROSE_BUFFER_THRESHOLD)
    }

    /// Create a new streaming buffer with a custom prose buffer threshold.
    ///
    /// # Arguments
    ///
    /// * `threshold` - Number of prose lines to buffer before flushing.
    ///   - `0` means emit each line immediately (eager streaming)
    ///   - `3-5` is a good balance for progressive streaming
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::chunk::StreamingChunkBuffer;
    ///
    /// // Buffer 3 lines of prose before emitting
    /// let buffer = StreamingChunkBuffer::with_prose_threshold(3);
    /// ```
    pub fn with_prose_threshold(threshold: usize) -> Self {
        Self {
            state: BufferState::Prose,
            buffer: String::new(),
            emitted_count: 0,
            prose_buffer_threshold: threshold,
            buffered_prose_lines: 0,
        }
    }

    /// Check if the buffer is empty (no pending content).
    ///
    /// Note: This only checks if there's buffered code content, not the parser state.
    /// Use [`is_in_code_block`](Self::is_in_code_block) to check if inside a code block.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::chunk::StreamingChunkBuffer;
    ///
    /// let mut buffer = StreamingChunkBuffer::new();
    /// assert!(buffer.is_empty());
    ///
    /// // Opening a code block clears the buffer for code content
    /// buffer.process_line("```rust");
    /// assert!(buffer.is_empty()); // Buffer is empty (no code content yet)
    /// assert!(buffer.is_in_code_block()); // But we are inside a code block
    ///
    /// // Adding code content makes the buffer non-empty
    /// buffer.process_line("fn main() {}");
    /// assert!(!buffer.is_empty()); // Now has buffered code content
    /// ```
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Check if currently inside a code/diff block.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::chunk::StreamingChunkBuffer;
    ///
    /// let mut buffer = StreamingChunkBuffer::new();
    /// assert!(!buffer.is_in_code_block());
    ///
    /// buffer.process_line("```rust");
    /// assert!(buffer.is_in_code_block());
    ///
    /// buffer.process_line("```");
    /// assert!(!buffer.is_in_code_block());
    /// ```
    pub fn is_in_code_block(&self) -> bool {
        matches!(self.state, BufferState::Code { .. })
    }

    /// Get the number of chunks emitted so far.
    pub fn emitted_count(&self) -> usize {
        self.emitted_count
    }

    /// Process a single line of input.
    ///
    /// Returns any complete chunks that were detected. Code blocks are buffered
    /// until their closing fence is seen. Prose streaming behavior depends on
    /// the `prose_buffer_threshold` setting:
    /// - `0`: Each prose line emitted immediately (eager streaming)
    /// - `N`: Prose lines buffered until N lines accumulated, then flushed as one chunk
    ///
    /// # Arguments
    ///
    /// * `line` - A single line of text (without trailing newline)
    ///
    /// # Returns
    ///
    /// A vector of complete chunks (may be empty if still buffering).
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::chunk::{StreamingChunkBuffer, ChunkType};
    ///
    /// let mut buffer = StreamingChunkBuffer::new();
    ///
    /// // Prose is emitted immediately (default threshold = 0)
    /// let chunks = buffer.process_line("Some text");
    /// assert_eq!(chunks.len(), 1);
    ///
    /// // Code blocks are buffered
    /// let chunks = buffer.process_line("```python");
    /// assert!(chunks.is_empty());
    ///
    /// let chunks = buffer.process_line("print('hello')");
    /// assert!(chunks.is_empty());
    ///
    /// // Emitted when fence closes
    /// let chunks = buffer.process_line("```");
    /// assert_eq!(chunks.len(), 1);
    /// ```
    pub fn process_line(&mut self, line: &str) -> Vec<ParsedChunk> {
        let mut result = Vec::new();

        match &self.state {
            BufferState::Prose => {
                // Check for opening fence
                if let Some(lang) = parse_fence_open(line) {
                    // Flush any buffered prose before starting a code block
                    if let Some(prose_chunk) = self.flush_prose_buffer() {
                        result.push(prose_chunk);
                    }

                    // Start a code block
                    let is_diff = lang.as_deref() == Some("diff");
                    self.state = BufferState::Code {
                        language: lang,
                        is_diff,
                    };
                    self.buffer.clear();
                } else {
                    // Handle prose based on buffering mode
                    if self.prose_buffer_threshold == 0 {
                        // Eager mode: emit each line immediately
                        // Emit all lines including empty ones to preserve whitespace
                        let chunk = ParsedChunk::prose(line);
                        self.emitted_count += 1;
                        result.push(chunk);
                    } else {
                        // Buffered mode: accumulate lines until threshold
                        if !self.buffer.is_empty() {
                            self.buffer.push('\n');
                        }
                        self.buffer.push_str(line);
                        self.buffered_prose_lines += 1;

                        // Check if we've reached the threshold
                        if self.buffered_prose_lines >= self.prose_buffer_threshold {
                            if let Some(prose_chunk) = self.flush_prose_buffer() {
                                result.push(prose_chunk);
                            }
                        }
                    }
                }
            }
            BufferState::Code { language, is_diff } => {
                // Check for closing fence
                if is_fence_close(line) {
                    // Emit the complete code/diff block
                    let content = std::mem::take(&mut self.buffer);
                    let chunk = if *is_diff {
                        ParsedChunk::diff(content)
                    } else {
                        ParsedChunk::code(content, language.clone())
                    };
                    self.emitted_count += 1;
                    result.push(chunk);

                    // Return to prose state
                    self.state = BufferState::Prose;
                } else {
                    // Accumulate code content
                    if !self.buffer.is_empty() {
                        self.buffer.push('\n');
                    }
                    self.buffer.push_str(line);
                }
            }
        }

        result
    }

    /// Flush the buffered prose lines and return as a single chunk.
    ///
    /// Returns `None` if no prose is buffered.
    fn flush_prose_buffer(&mut self) -> Option<ParsedChunk> {
        if self.buffered_prose_lines == 0 || self.buffer.is_empty() {
            return None;
        }

        let content = std::mem::take(&mut self.buffer);
        self.buffered_prose_lines = 0;
        self.emitted_count += 1;

        Some(ParsedChunk::prose(content))
    }

    /// Process multiple lines at once (convenience method).
    ///
    /// # Arguments
    ///
    /// * `text` - Text potentially containing multiple lines
    ///
    /// # Returns
    ///
    /// All chunks emitted while processing the text.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::chunk::StreamingChunkBuffer;
    ///
    /// let mut buffer = StreamingChunkBuffer::new();
    /// let text = "line1\nline2\nline3";
    /// let chunks = buffer.process_text(text);
    /// assert_eq!(chunks.len(), 3);
    /// ```
    pub fn process_text(&mut self, text: &str) -> Vec<ParsedChunk> {
        let mut result = Vec::new();
        // Use split_lines_preserve_trailing to handle trailing newlines correctly
        for line in split_lines_preserve_trailing(text) {
            result.extend(self.process_line(line));
        }
        result
    }

    /// Finish processing and return any remaining buffered content.
    ///
    /// This method should be called when the stream ends to handle any
    /// unterminated code blocks or buffered prose. After calling this, the buffer is reset.
    ///
    /// # Returns
    ///
    /// Any remaining chunks (e.g., unterminated code blocks, buffered prose).
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::chunk::{StreamingChunkBuffer, ChunkType};
    ///
    /// let mut buffer = StreamingChunkBuffer::new();
    ///
    /// // Unterminated code block
    /// buffer.process_line("```rust");
    /// buffer.process_line("fn main() {}");
    ///
    /// // finish() returns the incomplete block
    /// let final_chunks = buffer.finish();
    /// assert_eq!(final_chunks.len(), 1);
    /// assert!(matches!(final_chunks[0].chunk_type, ChunkType::Code { .. }));
    ///
    /// // Buffer is now empty and reset
    /// assert!(buffer.is_empty());
    /// ```
    ///
    /// # Example with Prose Buffering
    ///
    /// ```
    /// use ralph_core::chunk::{StreamingChunkBuffer, ChunkType};
    ///
    /// let mut buffer = StreamingChunkBuffer::with_prose_threshold(5);
    ///
    /// // Only 2 lines buffered (below threshold)
    /// buffer.process_line("Line 1");
    /// buffer.process_line("Line 2");
    ///
    /// // finish() flushes remaining prose
    /// let final_chunks = buffer.finish();
    /// assert_eq!(final_chunks.len(), 1);
    /// assert_eq!(final_chunks[0].content, "Line 1\nLine 2");
    /// ```
    pub fn finish(&mut self) -> Vec<ParsedChunk> {
        let mut result = Vec::new();

        match &self.state {
            BufferState::Prose => {
                // Flush any buffered prose (when using threshold-based buffering)
                if let Some(prose_chunk) = self.flush_prose_buffer() {
                    result.push(prose_chunk);
                }
            }
            BufferState::Code { language, is_diff } => {
                // Emit unterminated code block
                if !self.buffer.is_empty() {
                    let content = std::mem::take(&mut self.buffer);
                    let chunk = if *is_diff {
                        ParsedChunk::diff(content)
                    } else {
                        ParsedChunk::code(content, language.clone())
                    };
                    self.emitted_count += 1;
                    result.push(chunk);
                }
            }
        }

        // Reset state
        self.state = BufferState::Prose;
        self.buffer.clear();
        self.buffered_prose_lines = 0;

        result
    }

    /// Reset the buffer to its initial state, discarding any buffered content.
    ///
    /// Note: This preserves the prose buffer threshold setting.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::chunk::StreamingChunkBuffer;
    ///
    /// let mut buffer = StreamingChunkBuffer::new();
    /// buffer.process_line("```rust");
    /// buffer.process_line("some code");
    ///
    /// buffer.reset();
    /// assert!(buffer.is_empty());
    /// assert!(!buffer.is_in_code_block());
    /// ```
    pub fn reset(&mut self) {
        self.state = BufferState::Prose;
        self.buffer.clear();
        self.emitted_count = 0;
        self.buffered_prose_lines = 0;
    }

    /// Get the current prose buffer threshold.
    pub fn prose_buffer_threshold(&self) -> usize {
        self.prose_buffer_threshold
    }

    /// Set a new prose buffer threshold.
    ///
    /// Note: This does not flush any currently buffered content.
    /// Call [`finish()`](Self::finish) first if you want to preserve buffered prose.
    pub fn set_prose_threshold(&mut self, threshold: usize) {
        self.prose_buffer_threshold = threshold;
    }

    /// Get the number of currently buffered prose lines.
    pub fn buffered_prose_lines(&self) -> usize {
        self.buffered_prose_lines
    }

    /// Get the current buffered content (for debugging/inspection).
    pub fn buffered_content(&self) -> &str {
        &self.buffer
    }
}
