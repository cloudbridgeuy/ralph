//! Tests for prose buffer threshold (progressive streaming).

use crate::chunk::{ChunkType, StreamingChunkBuffer};

#[test]
fn test_prose_buffer_threshold_default_is_zero() {
    let buffer = StreamingChunkBuffer::new();
    assert_eq!(buffer.prose_buffer_threshold(), 0);
    assert_eq!(buffer.buffered_prose_lines(), 0);
}

#[test]
fn test_prose_buffer_threshold_with_value() {
    let buffer = StreamingChunkBuffer::with_prose_threshold(3);
    assert_eq!(buffer.prose_buffer_threshold(), 3);
}

#[test]
fn test_prose_buffer_eager_mode_emits_immediately() {
    // Default buffer (threshold=0) emits each line immediately
    let mut buffer = StreamingChunkBuffer::new();

    let chunks = buffer.process_line("Line 1");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "Line 1");

    let chunks = buffer.process_line("Line 2");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "Line 2");
}

#[test]
fn test_prose_buffer_buffered_mode_accumulates() {
    // Threshold of 3 means buffer until 3 lines
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(3);

    // First line - no emission, buffered
    let chunks = buffer.process_line("Line 1");
    assert!(chunks.is_empty());
    assert_eq!(buffer.buffered_prose_lines(), 1);

    // Second line - no emission, buffered
    let chunks = buffer.process_line("Line 2");
    assert!(chunks.is_empty());
    assert_eq!(buffer.buffered_prose_lines(), 2);

    // Third line - threshold reached, emit combined chunk
    let chunks = buffer.process_line("Line 3");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "Line 1\nLine 2\nLine 3");
    assert_eq!(buffer.buffered_prose_lines(), 0);
}

#[test]
fn test_prose_buffer_flushes_on_code_block_start() {
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(5);

    // Buffer 2 lines (below threshold)
    buffer.process_line("Line 1");
    buffer.process_line("Line 2");
    assert_eq!(buffer.buffered_prose_lines(), 2);

    // Opening fence should flush buffered prose first
    let chunks = buffer.process_line("```rust");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "Line 1\nLine 2");
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    assert!(buffer.is_in_code_block());
}

#[test]
fn test_prose_buffer_flushes_on_finish() {
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(5);

    // Buffer 2 lines (below threshold)
    buffer.process_line("Line 1");
    buffer.process_line("Line 2");
    assert_eq!(buffer.buffered_prose_lines(), 2);

    // finish() should flush remaining prose
    let final_chunks = buffer.finish();
    assert_eq!(final_chunks.len(), 1);
    assert_eq!(final_chunks[0].content, "Line 1\nLine 2");
}

#[test]
fn test_prose_buffer_empty_lines_count_towards_threshold() {
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(3);

    // Empty lines also count
    buffer.process_line("Line 1");
    buffer.process_line(""); // blank line
    let chunks = buffer.process_line("Line 2");

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "Line 1\n\nLine 2");
}

#[test]
fn test_prose_buffer_multiple_flushes() {
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(2);
    let mut all_chunks = Vec::new();

    // First batch - 2 lines
    all_chunks.extend(buffer.process_line("Line 1"));
    all_chunks.extend(buffer.process_line("Line 2")); // triggers flush

    // Second batch - 2 lines
    all_chunks.extend(buffer.process_line("Line 3"));
    all_chunks.extend(buffer.process_line("Line 4")); // triggers flush

    assert_eq!(all_chunks.len(), 2);
    assert_eq!(all_chunks[0].content, "Line 1\nLine 2");
    assert_eq!(all_chunks[1].content, "Line 3\nLine 4");
}

#[test]
fn test_prose_buffer_code_block_unchanged() {
    // Code blocks should still buffer fully regardless of prose threshold
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(2);

    buffer.process_line("```rust");
    buffer.process_line("line 1");
    buffer.process_line("line 2");
    buffer.process_line("line 3"); // would trigger prose threshold, but we're in code block
    let chunks = buffer.process_line("```");

    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Code { .. }));
    assert_eq!(chunks[0].content, "line 1\nline 2\nline 3");
}

#[test]
fn test_prose_buffer_set_threshold() {
    let mut buffer = StreamingChunkBuffer::new();
    assert_eq!(buffer.prose_buffer_threshold(), 0);

    buffer.set_prose_threshold(5);
    assert_eq!(buffer.prose_buffer_threshold(), 5);
}

#[test]
fn test_prose_buffer_reset_clears_buffered_lines() {
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(5);

    buffer.process_line("Line 1");
    buffer.process_line("Line 2");
    assert_eq!(buffer.buffered_prose_lines(), 2);

    buffer.reset();
    assert_eq!(buffer.buffered_prose_lines(), 0);
    // Threshold is preserved
    assert_eq!(buffer.prose_buffer_threshold(), 5);
}

#[test]
fn test_prose_buffer_mixed_content_integration() {
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(3);
    let mut all_chunks = Vec::new();

    // Prose (buffered)
    all_chunks.extend(buffer.process_line("Intro line 1"));
    all_chunks.extend(buffer.process_line("Intro line 2"));
    // Code block starts - flushes buffered prose
    all_chunks.extend(buffer.process_line("```rust"));
    all_chunks.extend(buffer.process_line("fn main() {}"));
    all_chunks.extend(buffer.process_line("```"));
    // More prose (buffered)
    all_chunks.extend(buffer.process_line("Outro line 1"));
    all_chunks.extend(buffer.process_line("Outro line 2"));
    // finish() flushes remaining
    all_chunks.extend(buffer.finish());

    assert_eq!(all_chunks.len(), 3);
    assert_eq!(all_chunks[0].content, "Intro line 1\nIntro line 2");
    assert!(matches!(all_chunks[0].chunk_type, ChunkType::Prose));
    assert!(matches!(all_chunks[1].chunk_type, ChunkType::Code { .. }));
    assert_eq!(all_chunks[2].content, "Outro line 1\nOutro line 2");
    assert!(matches!(all_chunks[2].chunk_type, ChunkType::Prose));
}

#[test]
fn test_prose_buffer_threshold_one_behaves_like_eager() {
    // Threshold of 1 should emit after each line (like eager mode)
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(1);

    let chunks = buffer.process_line("Line 1");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "Line 1");

    let chunks = buffer.process_line("Line 2");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "Line 2");
}

#[test]
fn test_prose_buffer_whitespace_preservation() {
    // Buffered prose should still preserve whitespace
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(3);

    buffer.process_line("  indented");
    buffer.process_line(""); // blank
    let chunks = buffer.process_line("trailing  ");

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "  indented\n\ntrailing  ");
}

#[test]
fn test_prose_buffer_diff_block_unchanged() {
    // Diff blocks should still buffer fully regardless of prose threshold
    let mut buffer = StreamingChunkBuffer::with_prose_threshold(2);

    buffer.process_line("```diff");
    buffer.process_line("-old");
    buffer.process_line("+new");
    buffer.process_line("-another");
    let chunks = buffer.process_line("```");

    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
    assert_eq!(chunks[0].content, "-old\n+new\n-another");
}
