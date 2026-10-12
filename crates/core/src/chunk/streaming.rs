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

#[derive(Debug, Clone, PartialEq)]
enum BufferState {
    Prose,
    Code {
        language: Option<String>,
        is_diff: bool,
    },
}

pub const DEFAULT_PROSE_BUFFER_THRESHOLD: usize = 0;

#[derive(Debug, Clone)]
pub struct StreamingChunkBuffer {
    state: BufferState,
    buffer: String,
    prose_buffer_threshold: usize,
    buffered_prose_lines: usize,
}

impl Default for StreamingChunkBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingChunkBuffer {
    pub fn new() -> Self {
        Self::with_prose_threshold(DEFAULT_PROSE_BUFFER_THRESHOLD)
    }

    pub fn with_prose_threshold(threshold: usize) -> Self {
        Self {
            state: BufferState::Prose,
            buffer: String::new(),
            prose_buffer_threshold: threshold,
            buffered_prose_lines: 0,
        }
    }

    pub fn process_line(&mut self, line: &str) -> Vec<ParsedChunk> {
        match &self.state {
            BufferState::Prose => {
                if let Some(lang) = parse_fence_open(line) {
                    self.start_code_block(lang)
                } else if self.prose_buffer_threshold == 0 {
                    vec![ParsedChunk::prose(line)]
                } else {
                    self.accumulate_prose_line(line)
                }
            }
            BufferState::Code { .. } => {
                if is_fence_close(line) {
                    self.close_code_block()
                } else {
                    self.accumulate_code_line(line);
                    Vec::new()
                }
            }
        }
    }

    fn flush_prose_buffer(&mut self) -> Option<ParsedChunk> {
        if self.buffered_prose_lines == 0 || self.buffer.is_empty() {
            return None;
        }

        let content = std::mem::take(&mut self.buffer);
        self.buffered_prose_lines = 0;

        Some(ParsedChunk::prose(content))
    }

    fn accumulate_prose_line(&mut self, line: &str) -> Vec<ParsedChunk> {
        if !self.buffer.is_empty() {
            self.buffer.push('\n');
        }
        self.buffer.push_str(line);
        self.buffered_prose_lines += 1;

        if self.buffered_prose_lines >= self.prose_buffer_threshold {
            self.flush_prose_buffer().into_iter().collect()
        } else {
            Vec::new()
        }
    }

    fn start_code_block(&mut self, language: Option<String>) -> Vec<ParsedChunk> {
        let mut result = Vec::new();

        if let Some(prose_chunk) = self.flush_prose_buffer() {
            result.push(prose_chunk);
        }

        let is_diff = language.as_deref() == Some("diff");
        self.state = BufferState::Code { language, is_diff };
        self.buffer.clear();

        result
    }

    fn close_code_block(&mut self) -> Vec<ParsedChunk> {
        let (language, is_diff) = match &self.state {
            BufferState::Code { language, is_diff } => (language.clone(), *is_diff),
            BufferState::Prose => return Vec::new(),
        };

        let content = std::mem::take(&mut self.buffer);
        let chunk = if is_diff {
            ParsedChunk::diff(content)
        } else {
            ParsedChunk::code(content, language)
        };
        self.state = BufferState::Prose;

        vec![chunk]
    }

    fn accumulate_code_line(&mut self, line: &str) {
        if !self.buffer.is_empty() {
            self.buffer.push('\n');
        }
        self.buffer.push_str(line);
    }

    pub fn finish(&mut self) -> Vec<ParsedChunk> {
        let mut result = Vec::new();

        match &self.state {
            BufferState::Prose => {
                if let Some(prose_chunk) = self.flush_prose_buffer() {
                    result.push(prose_chunk);
                }
            }
            BufferState::Code { .. } => {
                if !self.buffer.is_empty() {
                    result.extend(self.close_code_block());
                }
            }
        }

        self.state = BufferState::Prose;
        self.buffer.clear();
        self.buffered_prose_lines = 0;

        result
    }
}
