//! Streaming chunk parser for incremental processing.

use super::fence::{is_fence_close, parse_fence_open};
use super::types::ParsedChunk;

/// Internal state for the streaming chunk buffer.
#[derive(Debug, Clone, PartialEq)]
enum BufferState {
    /// Processing prose content (emit lines eagerly).
    Prose,
    /// Inside a fenced code block (buffer until closing fence).
    Code {
        language: Option<String>,
        is_diff: bool,
    },
}

/// A streaming buffer for parsing LLM output into typed chunks.
///
/// This buffer processes text line by line, emitting complete chunks as soon
/// as their boundaries are detected. Code blocks and diff blocks are buffered
/// until their closing fence is seen, while prose streams more eagerly.
///
/// # Design Principles
///
/// - **Code blocks buffer**: Not emitted until closing ``` is seen
/// - **Diff blocks buffer**: Treated the same as code blocks (```diff fence)
/// - **Prose streams eagerly**: Each line emitted as its own chunk
/// - **Final flush**: Unterminated blocks are emitted on [`finish()`](Self::finish)
///
/// # Example
///
/// ```
/// use ralph_core::chunk::{StreamingChunkBuffer, ChunkType};
///
/// let mut buffer = StreamingChunkBuffer::new();
///
/// // Prose emitted eagerly
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
#[derive(Debug, Clone)]
pub struct StreamingChunkBuffer {
    /// Current parser state.
    state: BufferState,
    /// Accumulated content for the current chunk.
    buffer: String,
    /// Count of emitted chunks (for debugging).
    emitted_count: usize,
}

impl Default for StreamingChunkBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingChunkBuffer {
    /// Create a new empty streaming buffer.
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
        Self {
            state: BufferState::Prose,
            buffer: String::new(),
            emitted_count: 0,
        }
    }

    /// Check if the buffer is empty (no pending content).
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::chunk::StreamingChunkBuffer;
    ///
    /// let mut buffer = StreamingChunkBuffer::new();
    /// assert!(buffer.is_empty());
    ///
    /// buffer.process_line("```rust");
    /// assert!(!buffer.is_empty()); // Code block started
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
    /// until their closing fence is seen. Prose lines are emitted immediately.
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
    /// // Prose is emitted immediately
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
                    // Start a code block
                    let is_diff = lang.as_deref() == Some("diff");
                    self.state = BufferState::Code {
                        language: lang,
                        is_diff,
                    };
                    self.buffer.clear();
                } else {
                    // Emit prose line immediately (eager streaming)
                    // Only emit non-empty lines to avoid noise
                    if !line.is_empty() {
                        let chunk = ParsedChunk::prose(line);
                        self.emitted_count += 1;
                        result.push(chunk);
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
        for line in text.lines() {
            result.extend(self.process_line(line));
        }
        result
    }

    /// Finish processing and return any remaining buffered content.
    ///
    /// This method should be called when the stream ends to handle any
    /// unterminated code blocks. After calling this, the buffer is reset.
    ///
    /// # Returns
    ///
    /// Any remaining chunks (e.g., unterminated code blocks).
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
    pub fn finish(&mut self) -> Vec<ParsedChunk> {
        let mut result = Vec::new();

        match &self.state {
            BufferState::Prose => {
                // Nothing to flush for prose (lines already emitted)
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

        result
    }

    /// Reset the buffer to its initial state, discarding any buffered content.
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
    }

    /// Get the current buffered content (for debugging/inspection).
    pub fn buffered_content(&self) -> &str {
        &self.buffer
    }
}
