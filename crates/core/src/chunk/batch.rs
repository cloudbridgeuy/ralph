//! Batch parsing for complete text.

use super::fence::{is_fence_close, parse_fence_open};
use super::types::ParsedChunk;

/// Flush accumulated prose into the chunks vector if non-empty.
///
/// This is a pure helper that takes ownership of the prose content.
fn flush_prose(chunks: &mut Vec<ParsedChunk>, prose: &mut String) {
    if !prose.is_empty() {
        chunks.push(ParsedChunk::prose(std::mem::take(prose)));
    }
}

/// Emit a completed code block chunk.
///
/// This is a pure helper that takes ownership of the code content.
fn emit_code_block(
    chunks: &mut Vec<ParsedChunk>,
    content: &mut String,
    language: &mut Option<String>,
    is_diff: bool,
) {
    let code_content = std::mem::take(content);
    if is_diff {
        chunks.push(ParsedChunk::diff(code_content));
    } else {
        chunks.push(ParsedChunk::code(code_content, language.take()));
    }
}

/// Append a line to a buffer, adding newline separator if buffer is non-empty.
fn append_line(buffer: &mut String, line: &str) {
    if !buffer.is_empty() {
        buffer.push('\n');
    }
    buffer.push_str(line);
}

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
            if let Some(lang) = parse_fence_open(line) {
                flush_prose(&mut chunks, &mut current_prose);
                in_code_block = true;
                is_diff_block = lang.as_deref() == Some("diff");
                code_block_language = lang;
                code_block_content.clear();
            } else {
                append_line(&mut current_prose, line);
            }
        } else if is_fence_close(line) {
            emit_code_block(
                &mut chunks,
                &mut code_block_content,
                &mut code_block_language,
                is_diff_block,
            );
            in_code_block = false;
            is_diff_block = false;
        } else {
            append_line(&mut code_block_content, line);
        }
    }

    // Handle unterminated code block
    if in_code_block && !code_block_content.is_empty() {
        emit_code_block(
            &mut chunks,
            &mut code_block_content,
            &mut code_block_language,
            is_diff_block,
        );
    }

    // Flush remaining prose
    flush_prose(&mut chunks, &mut current_prose);

    chunks
}
