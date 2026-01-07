//! Core tests for StreamingChunkBuffer.

use crate::chunk::{ChunkType, StreamingChunkBuffer};

#[test]
fn test_streaming_buffer_new() {
    let buffer = StreamingChunkBuffer::new();
    assert!(buffer.is_empty());
    assert!(!buffer.is_in_code_block());
    assert_eq!(buffer.emitted_count(), 0);
}

#[test]
fn test_streaming_buffer_default() {
    let buffer = StreamingChunkBuffer::default();
    assert!(buffer.is_empty());
}

#[test]
fn test_streaming_prose_emits_immediately() {
    let mut buffer = StreamingChunkBuffer::new();

    let chunks = buffer.process_line("Hello, world!");
    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    assert_eq!(chunks[0].content, "Hello, world!");

    // Buffer should be empty after prose emission
    assert!(buffer.is_empty());
    assert_eq!(buffer.emitted_count(), 1);
}

#[test]
fn test_streaming_code_block_buffers() {
    let mut buffer = StreamingChunkBuffer::new();

    // Opening fence - no emission, starts buffering
    let chunks = buffer.process_line("```rust");
    assert!(chunks.is_empty());
    assert!(buffer.is_in_code_block());

    // Code content - no emission
    let chunks = buffer.process_line("fn main() {}");
    assert!(chunks.is_empty());
    assert!(!buffer.is_empty()); // Has buffered content

    // Closing fence - emits complete block
    let chunks = buffer.process_line("```");
    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(chunks[0].content, "fn main() {}");

    // Back to prose mode
    assert!(!buffer.is_in_code_block());
    assert!(buffer.is_empty());
}

#[test]
fn test_streaming_diff_block_buffers() {
    let mut buffer = StreamingChunkBuffer::new();

    let chunks = buffer.process_line("```diff");
    assert!(chunks.is_empty());
    assert!(buffer.is_in_code_block());

    let chunks = buffer.process_line("-old");
    assert!(chunks.is_empty());

    let chunks = buffer.process_line("+new");
    assert!(chunks.is_empty());

    let chunks = buffer.process_line("```");
    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
    assert_eq!(chunks[0].content, "-old\n+new");
}

#[test]
fn test_streaming_code_block_without_language() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```");
    buffer.process_line("some code");
    let chunks = buffer.process_line("```");

    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert!(language.is_none());
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(chunks[0].content, "some code");
}

#[test]
fn test_streaming_finish_unterminated_code_block() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```rust");
    buffer.process_line("fn main() {}");
    buffer.process_line("// more code");

    // finish() should return the unterminated block
    let final_chunks = buffer.finish();
    assert_eq!(final_chunks.len(), 1);
    match &final_chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(final_chunks[0].content, "fn main() {}\n// more code");

    // Buffer should be reset after finish
    assert!(buffer.is_empty());
    assert!(!buffer.is_in_code_block());
}

#[test]
fn test_streaming_finish_unterminated_diff_block() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```diff");
    buffer.process_line("-removed");
    buffer.process_line("+added");

    let final_chunks = buffer.finish();
    assert_eq!(final_chunks.len(), 1);
    assert!(matches!(final_chunks[0].chunk_type, ChunkType::Diff));
}

#[test]
fn test_streaming_finish_prose_mode() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("Some prose");

    // finish() in prose mode returns nothing (prose already emitted)
    let final_chunks = buffer.finish();
    assert!(final_chunks.is_empty());
}

#[test]
fn test_streaming_empty_lines_emitted_for_whitespace_preservation() {
    let mut buffer = StreamingChunkBuffer::new();

    // Empty lines ARE emitted to preserve whitespace between paragraphs
    let chunks = buffer.process_line("");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "");
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
}

#[test]
fn test_streaming_whitespace_lines_emitted() {
    let mut buffer = StreamingChunkBuffer::new();

    // Whitespace-only lines are emitted (they're non-empty)
    let chunks = buffer.process_line("   ");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "   ");
}

#[test]
fn test_streaming_mixed_content() {
    let mut buffer = StreamingChunkBuffer::new();
    let mut all_chunks = Vec::new();

    // Prose
    all_chunks.extend(buffer.process_line("Here's some code:"));

    // Code block
    all_chunks.extend(buffer.process_line("```rust"));
    all_chunks.extend(buffer.process_line("fn main() {}"));
    all_chunks.extend(buffer.process_line("```"));

    // More prose
    all_chunks.extend(buffer.process_line("That's the code."));

    // Final flush
    all_chunks.extend(buffer.finish());

    assert_eq!(all_chunks.len(), 3);
    assert!(matches!(all_chunks[0].chunk_type, ChunkType::Prose));
    assert!(matches!(all_chunks[1].chunk_type, ChunkType::Code { .. }));
    assert!(matches!(all_chunks[2].chunk_type, ChunkType::Prose));
}

#[test]
fn test_streaming_process_text() {
    let mut buffer = StreamingChunkBuffer::new();

    let text = "line1\nline2\nline3";
    let chunks = buffer.process_text(text);

    assert_eq!(chunks.len(), 3);
    assert!(chunks
        .iter()
        .all(|c| matches!(c.chunk_type, ChunkType::Prose)));
}

#[test]
fn test_streaming_process_text_with_code_block() {
    let mut buffer = StreamingChunkBuffer::new();

    let text = "intro\n```rust\ncode\n```\noutro";
    let chunks = buffer.process_text(text);

    assert_eq!(chunks.len(), 3);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    assert!(matches!(chunks[1].chunk_type, ChunkType::Code { .. }));
    assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
}

#[test]
fn test_streaming_reset() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```rust");
    buffer.process_line("some code");
    assert!(!buffer.is_empty());
    assert!(buffer.is_in_code_block());

    buffer.reset();
    assert!(buffer.is_empty());
    assert!(!buffer.is_in_code_block());
    assert_eq!(buffer.emitted_count(), 0);
}

#[test]
fn test_streaming_buffered_content() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```rust");
    buffer.process_line("line1");
    buffer.process_line("line2");

    assert_eq!(buffer.buffered_content(), "line1\nline2");
}

#[test]
fn test_streaming_multiline_code() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```python");
    buffer.process_line("def hello():");
    buffer.process_line("    print('hello')");
    buffer.process_line("    print('world')");
    let chunks = buffer.process_line("```");

    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].content,
        "def hello():\n    print('hello')\n    print('world')"
    );
}

#[test]
fn test_streaming_multiple_code_blocks() {
    let mut buffer = StreamingChunkBuffer::new();
    let mut all_chunks = Vec::new();

    // First code block
    all_chunks.extend(buffer.process_line("```rust"));
    all_chunks.extend(buffer.process_line("fn a() {}"));
    all_chunks.extend(buffer.process_line("```"));

    // Second code block
    all_chunks.extend(buffer.process_line("```python"));
    all_chunks.extend(buffer.process_line("def b(): pass"));
    all_chunks.extend(buffer.process_line("```"));

    assert_eq!(all_chunks.len(), 2);

    match &all_chunks[0].chunk_type {
        ChunkType::Code { language } => assert_eq!(language.as_deref(), Some("rust")),
        _ => panic!("Expected code chunk"),
    }

    match &all_chunks[1].chunk_type {
        ChunkType::Code { language } => assert_eq!(language.as_deref(), Some("python")),
        _ => panic!("Expected code chunk"),
    }
}

#[test]
fn test_streaming_emitted_count_tracking() {
    let mut buffer = StreamingChunkBuffer::new();

    assert_eq!(buffer.emitted_count(), 0);

    buffer.process_line("line1");
    assert_eq!(buffer.emitted_count(), 1);

    buffer.process_line("line2");
    assert_eq!(buffer.emitted_count(), 2);

    buffer.process_line("```rust");
    buffer.process_line("code");
    buffer.process_line("```");
    assert_eq!(buffer.emitted_count(), 3);
}

#[test]
fn test_streaming_indented_fence() {
    let mut buffer = StreamingChunkBuffer::new();

    // Indented fences should still be detected
    buffer.process_line("  ```rust");
    assert!(buffer.is_in_code_block());

    buffer.process_line("  code");
    let chunks = buffer.process_line("  ```");

    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
}

#[test]
fn test_streaming_empty_code_block() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```rust");
    let chunks = buffer.process_line("```");

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "");
}

#[test]
fn test_streaming_finish_empty_code_block() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```rust");
    // Don't add any content, just finish

    let final_chunks = buffer.finish();
    // Empty unterminated code block - nothing to emit
    assert!(final_chunks.is_empty());
}
