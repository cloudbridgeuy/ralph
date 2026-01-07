//! Tests for chunk parsing functionality.

use super::*;

#[test]
fn test_parse_simple_prose() {
    let text = "Hello, world!\nThis is prose.";
    let chunks = parse_chunks(text);

    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    assert_eq!(chunks[0].content, "Hello, world!\nThis is prose.");
}

#[test]
fn test_parse_code_block_with_language() {
    let text = "Here's some code:\n\n```rust\nfn main() {}\n```\n\nThat's it.";
    let chunks = parse_chunks(text);

    assert_eq!(chunks.len(), 3);

    // First prose
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    assert!(chunks[0].content.contains("Here's some code:"));

    // Code block
    match &chunks[1].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(chunks[1].content, "fn main() {}");

    // Final prose
    assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
    assert!(chunks[2].content.contains("That's it."));
}

#[test]
fn test_parse_code_block_without_language() {
    let text = "```\nsome code\n```";
    let chunks = parse_chunks(text);

    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(*language, None);
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(chunks[0].content, "some code");
}

#[test]
fn test_parse_diff_block() {
    let text = "Changes:\n\n```diff\n-old\n+new\n```\n\nDone.";
    let chunks = parse_chunks(text);

    assert_eq!(chunks.len(), 3);

    // First prose
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));

    // Diff block
    assert!(matches!(chunks[1].chunk_type, ChunkType::Diff));
    assert_eq!(chunks[1].content, "-old\n+new");

    // Final prose
    assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
}

#[test]
fn test_parse_multiple_code_blocks() {
    let text = "```python\nprint('a')\n```\n\nand\n\n```javascript\nconsole.log('b')\n```";
    let chunks = parse_chunks(text);

    assert_eq!(chunks.len(), 3);

    // First code block
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("python"));
        }
        _ => panic!("Expected code chunk"),
    }

    // Prose between
    assert!(matches!(chunks[1].chunk_type, ChunkType::Prose));

    // Second code block
    match &chunks[2].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("javascript"));
        }
        _ => panic!("Expected code chunk"),
    }
}

#[test]
fn test_parse_unterminated_code_block() {
    let text = "```rust\nfn main() {}";
    let chunks = parse_chunks(text);

    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(chunks[0].content, "fn main() {}");
}

#[test]
fn test_parse_empty_text() {
    let chunks = parse_chunks("");
    assert!(chunks.is_empty());
}

#[test]
fn test_parse_only_whitespace() {
    let chunks = parse_chunks("   \n\n   ");
    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
}

#[test]
fn test_is_unfenced_diff_with_git_diff() {
    let text = "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs";
    assert!(is_unfenced_diff(text));
}

#[test]
fn test_is_unfenced_diff_with_hunk_header() {
    let text = "@@ -1,3 +1,4 @@\n fn main() {\n+    println!(\"Hello\");\n }";
    assert!(is_unfenced_diff(text));
}

#[test]
fn test_is_unfenced_diff_with_plus_minus_lines() {
    let text = "-old line\n+new line";
    assert!(is_unfenced_diff(text));
}

#[test]
fn test_is_unfenced_diff_regular_text() {
    let text = "This is just regular text.\nNothing special here.";
    assert!(!is_unfenced_diff(text));
}

#[test]
fn test_parse_chunks_with_heuristics_fenced() {
    let text = "```rust\nfn main() {}\n```";
    let chunks = parse_chunks_with_heuristics(text);

    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
}

#[test]
fn test_parse_chunks_with_heuristics_unfenced_diff() {
    let text =
        "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new";
    let chunks = parse_chunks_with_heuristics(text);

    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
}

#[test]
fn test_parsed_chunk_constructors() {
    let prose = ParsedChunk::prose("hello");
    assert!(matches!(prose.chunk_type, ChunkType::Prose));
    assert_eq!(prose.content, "hello");

    let code = ParsedChunk::code("fn main()", Some("rust".to_string()));
    match &code.chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(code.content, "fn main()");

    let diff = ParsedChunk::diff("-old\n+new");
    assert!(matches!(diff.chunk_type, ChunkType::Diff));
    assert_eq!(diff.content, "-old\n+new");
}

#[test]
fn test_chunk_type_serialization() {
    // Test prose serialization
    let prose = ParsedChunk::prose("hello");
    let json = serde_json::to_string(&prose).unwrap();
    assert!(json.contains(r#""type":"prose""#));

    // Test code serialization with language
    let code = ParsedChunk::code("fn main()", Some("rust".to_string()));
    let json = serde_json::to_string(&code).unwrap();
    assert!(json.contains(r#""type":"code""#));
    assert!(json.contains(r#""language":"rust""#));

    // Test code serialization without language
    let code_no_lang = ParsedChunk::code("fn main()", None);
    let json = serde_json::to_string(&code_no_lang).unwrap();
    assert!(json.contains(r#""type":"code""#));
    assert!(!json.contains("language"));

    // Test diff serialization
    let diff = ParsedChunk::diff("-old\n+new");
    let json = serde_json::to_string(&diff).unwrap();
    assert!(json.contains(r#""type":"diff""#));
}

#[test]
fn test_chunk_type_deserialization() {
    let json = r#"{"type":"code","language":"python","content":"print(1)"}"#;
    let chunk: ParsedChunk = serde_json::from_str(json).unwrap();

    match &chunk.chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("python"));
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(chunk.content, "print(1)");
}

#[test]
fn test_parse_indented_fence() {
    let text = "Example:\n  ```rust\n  fn main() {}\n  ```";
    let chunks = parse_chunks(text);

    // Should detect the indented fence
    assert_eq!(chunks.len(), 2);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));

    // Note: content preserves indentation from inside the block
    match &chunks[1].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
}

#[test]
fn test_parse_fence_with_extra_info() {
    // Some markdown allows extra metadata after the language
    let text = "```rust ignore\nfn main() {}\n```";
    let chunks = parse_chunks(text);

    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            // Should only capture "rust", not "rust ignore"
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
}

#[test]
fn test_roundtrip_serialization() {
    let original = vec![
        ParsedChunk::prose("intro"),
        ParsedChunk::code("fn main()", Some("rust".to_string())),
        ParsedChunk::diff("-a\n+b"),
    ];

    let json = serde_json::to_string(&original).unwrap();
    let deserialized: Vec<ParsedChunk> = serde_json::from_str(&json).unwrap();

    assert_eq!(original, deserialized);
}

#[test]
fn test_adjacent_code_blocks_no_prose() {
    // When code blocks are immediately adjacent (no prose between),
    // the parser produces just the code blocks without empty prose chunks.
    let text = "```rust\nfn a() {}\n```\n```python\ndef b():\n    pass\n```";
    let chunks = parse_chunks(text);

    assert_eq!(chunks.len(), 2);

    // First code block
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }

    // Second code block (immediately follows first)
    match &chunks[1].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("python"));
        }
        _ => panic!("Expected code chunk"),
    }
}

#[test]
fn test_multiline_code_block() {
    let text = "```rust\nfn main() {\n    println!(\"Hello\");\n    println!(\"World\");\n}\n```";
    let chunks = parse_chunks(text);

    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(
        chunks[0].content,
        "fn main() {\n    println!(\"Hello\");\n    println!(\"World\");\n}"
    );
}

// ==========================================================================
// StreamingChunkBuffer tests
// ==========================================================================

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
fn test_streaming_empty_lines_not_emitted() {
    let mut buffer = StreamingChunkBuffer::new();

    // Empty lines are not emitted
    let chunks = buffer.process_line("");
    assert!(chunks.is_empty());
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
