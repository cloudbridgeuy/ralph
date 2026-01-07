//! Batch parsing for complete text.

use super::fence::{is_fence_close, parse_fence_open};
use super::types::ParsedChunk;

/// Parse text into typed chunks.
///
/// This function scans text for fenced code blocks and diff patterns,
/// extracting them as typed chunks while preserving prose content between them.
///
/// # Arguments
///
/// * `text` - The accumulated text from assistant events
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
///
/// # Example
///
/// ```
/// use ralph_core::chunk::{parse_chunks, ChunkType};
///
/// let text = "Here's code:\n\n```rust\nfn main() {}\n```\n\nDone.";
/// let chunks = parse_chunks(text);
/// assert_eq!(chunks.len(), 3);
/// assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
/// assert!(matches!(chunks[1].chunk_type, ChunkType::Code { .. }));
/// assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
/// ```
pub fn parse_chunks(text: &str) -> Vec<ParsedChunk> {
    let mut chunks = Vec::new();
    let mut current_prose = String::new();
    let mut in_code_block = false;
    let mut code_block_content = String::new();
    let mut code_block_language: Option<String> = None;
    let mut is_diff_block = false;

    for line in text.lines() {
        if !in_code_block {
            // Check for opening fence
            if let Some(lang) = parse_fence_open(line) {
                // Flush any accumulated prose
                if !current_prose.is_empty() {
                    chunks.push(ParsedChunk::prose(std::mem::take(&mut current_prose)));
                }

                in_code_block = true;
                is_diff_block = lang.as_deref() == Some("diff");
                code_block_language = lang;
                code_block_content.clear();
            } else {
                // Accumulate prose
                if !current_prose.is_empty() {
                    current_prose.push('\n');
                }
                current_prose.push_str(line);
            }
        } else {
            // Inside a code block
            if is_fence_close(line) {
                // End of code block
                if is_diff_block {
                    chunks.push(ParsedChunk::diff(std::mem::take(&mut code_block_content)));
                } else {
                    chunks.push(ParsedChunk::code(
                        std::mem::take(&mut code_block_content),
                        code_block_language.take(),
                    ));
                }
                in_code_block = false;
                is_diff_block = false;
            } else {
                // Accumulate code content
                if !code_block_content.is_empty() {
                    code_block_content.push('\n');
                }
                code_block_content.push_str(line);
            }
        }
    }

    // Handle unterminated code block
    if in_code_block && !code_block_content.is_empty() {
        if is_diff_block {
            chunks.push(ParsedChunk::diff(code_block_content));
        } else {
            chunks.push(ParsedChunk::code(code_block_content, code_block_language));
        }
    }

    // Flush remaining prose
    if !current_prose.is_empty() {
        chunks.push(ParsedChunk::prose(current_prose));
    }

    chunks
}
