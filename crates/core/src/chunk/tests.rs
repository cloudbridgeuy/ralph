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

// ==========================================================================
// Whitespace preservation tests
// ==========================================================================

#[test]
fn test_whitespace_blank_lines_between_paragraphs() {
    let mut buffer = StreamingChunkBuffer::new();
    let mut all_chunks = Vec::new();

    // Simulate: "Paragraph 1.\n\nParagraph 2."
    all_chunks.extend(buffer.process_line("Paragraph 1."));
    all_chunks.extend(buffer.process_line("")); // blank line
    all_chunks.extend(buffer.process_line("Paragraph 2."));

    assert_eq!(all_chunks.len(), 3);
    assert_eq!(all_chunks[0].content, "Paragraph 1.");
    assert_eq!(all_chunks[1].content, ""); // blank line preserved
    assert_eq!(all_chunks[2].content, "Paragraph 2.");
}

#[test]
fn test_whitespace_multiple_blank_lines() {
    let mut buffer = StreamingChunkBuffer::new();
    let mut all_chunks = Vec::new();

    // Multiple blank lines should all be preserved
    all_chunks.extend(buffer.process_line("Text"));
    all_chunks.extend(buffer.process_line(""));
    all_chunks.extend(buffer.process_line(""));
    all_chunks.extend(buffer.process_line(""));
    all_chunks.extend(buffer.process_line("More text"));

    assert_eq!(all_chunks.len(), 5);
    assert_eq!(all_chunks[1].content, "");
    assert_eq!(all_chunks[2].content, "");
    assert_eq!(all_chunks[3].content, "");
}

#[test]
fn test_whitespace_code_block_indentation_preserved() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```python");
    buffer.process_line("def foo():");
    buffer.process_line("    x = 1");
    buffer.process_line("        nested = 2");
    buffer.process_line("    return x");
    let chunks = buffer.process_line("```");

    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].content,
        "def foo():\n    x = 1\n        nested = 2\n    return x"
    );
}

#[test]
fn test_whitespace_empty_lines_in_code_block() {
    let mut buffer = StreamingChunkBuffer::new();

    buffer.process_line("```rust");
    buffer.process_line("fn a() {}");
    buffer.process_line(""); // empty line in code
    buffer.process_line("fn b() {}");
    let chunks = buffer.process_line("```");

    assert_eq!(chunks.len(), 1);
    // Empty line should be preserved in code block
    assert_eq!(chunks[0].content, "fn a() {}\n\nfn b() {}");
}

#[test]
fn test_whitespace_blank_lines_before_code_block() {
    let mut buffer = StreamingChunkBuffer::new();
    let mut all_chunks = Vec::new();

    all_chunks.extend(buffer.process_line("Here's code:"));
    all_chunks.extend(buffer.process_line("")); // blank line before code
    all_chunks.extend(buffer.process_line("```rust"));
    all_chunks.extend(buffer.process_line("fn main() {}"));
    all_chunks.extend(buffer.process_line("```"));

    assert_eq!(all_chunks.len(), 3);
    assert_eq!(all_chunks[0].content, "Here's code:");
    assert_eq!(all_chunks[1].content, ""); // blank line preserved
    assert!(matches!(all_chunks[2].chunk_type, ChunkType::Code { .. }));
}

#[test]
fn test_whitespace_blank_lines_after_code_block() {
    let mut buffer = StreamingChunkBuffer::new();
    let mut all_chunks = Vec::new();

    all_chunks.extend(buffer.process_line("```rust"));
    all_chunks.extend(buffer.process_line("fn main() {}"));
    all_chunks.extend(buffer.process_line("```"));
    all_chunks.extend(buffer.process_line("")); // blank line after code
    all_chunks.extend(buffer.process_line("Done."));

    assert_eq!(all_chunks.len(), 3);
    assert!(matches!(all_chunks[0].chunk_type, ChunkType::Code { .. }));
    assert_eq!(all_chunks[1].content, ""); // blank line preserved
    assert_eq!(all_chunks[2].content, "Done.");
}

#[test]
fn test_whitespace_leading_spaces_in_prose() {
    let mut buffer = StreamingChunkBuffer::new();

    // Leading spaces in prose should be preserved
    let chunks = buffer.process_line("    indented text");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "    indented text");
}

#[test]
fn test_whitespace_trailing_spaces_in_prose() {
    let mut buffer = StreamingChunkBuffer::new();

    // Trailing spaces should be preserved (markdown uses them for line breaks)
    let chunks = buffer.process_line("text with trailing  ");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "text with trailing  ");
}

#[test]
fn test_whitespace_list_indentation() {
    let mut buffer = StreamingChunkBuffer::new();
    let mut all_chunks = Vec::new();

    // Simulate a nested list
    all_chunks.extend(buffer.process_line("- Item 1"));
    all_chunks.extend(buffer.process_line("  - Nested item"));
    all_chunks.extend(buffer.process_line("    - Deeply nested"));
    all_chunks.extend(buffer.process_line("- Item 2"));

    assert_eq!(all_chunks.len(), 4);
    assert_eq!(all_chunks[0].content, "- Item 1");
    assert_eq!(all_chunks[1].content, "  - Nested item");
    assert_eq!(all_chunks[2].content, "    - Deeply nested");
    assert_eq!(all_chunks[3].content, "- Item 2");
}

#[test]
fn test_whitespace_process_text_preserves_blank_lines() {
    let mut buffer = StreamingChunkBuffer::new();

    let text = "Line 1\n\nLine 2\n\n\nLine 3";
    let chunks = buffer.process_text(text);

    // Should have: Line 1, empty, Line 2, empty, empty, Line 3
    assert_eq!(chunks.len(), 6);
    assert_eq!(chunks[0].content, "Line 1");
    assert_eq!(chunks[1].content, "");
    assert_eq!(chunks[2].content, "Line 2");
    assert_eq!(chunks[3].content, "");
    assert_eq!(chunks[4].content, "");
    assert_eq!(chunks[5].content, "Line 3");
}

// ==========================================================================
// split_lines_preserve_trailing tests
// ==========================================================================

#[test]
fn test_split_lines_basic() {
    use super::split_lines_preserve_trailing;

    let lines: Vec<_> = split_lines_preserve_trailing("a\nb").collect();
    assert_eq!(lines, vec!["a", "b"]);
}

#[test]
fn test_split_lines_trailing_newline() {
    use super::split_lines_preserve_trailing;

    // With trailing newline - should include trailing empty string
    let lines: Vec<_> = split_lines_preserve_trailing("a\nb\n").collect();
    assert_eq!(lines, vec!["a", "b", ""]);
}

#[test]
fn test_split_lines_multiple_trailing_newlines() {
    use super::split_lines_preserve_trailing;

    // Multiple trailing newlines
    let lines: Vec<_> = split_lines_preserve_trailing("a\n\n").collect();
    assert_eq!(lines, vec!["a", "", ""]);
}

#[test]
fn test_split_lines_empty_string() {
    use super::split_lines_preserve_trailing;

    let lines: Vec<_> = split_lines_preserve_trailing("").collect();
    assert!(lines.is_empty());
}

#[test]
fn test_split_lines_single_newline() {
    use super::split_lines_preserve_trailing;

    // Just a newline
    let lines: Vec<_> = split_lines_preserve_trailing("\n").collect();
    assert_eq!(lines, vec!["", ""]);
}

#[test]
fn test_split_lines_no_newline() {
    use super::split_lines_preserve_trailing;

    let lines: Vec<_> = split_lines_preserve_trailing("hello").collect();
    assert_eq!(lines, vec!["hello"]);
}

#[test]
fn test_split_lines_blank_lines_middle() {
    use super::split_lines_preserve_trailing;

    // Blank lines in the middle
    let lines: Vec<_> = split_lines_preserve_trailing("a\n\nb").collect();
    assert_eq!(lines, vec!["a", "", "b"]);
}

#[test]
fn test_split_lines_crlf() {
    use super::split_lines_preserve_trailing;

    // CRLF line endings (Windows)
    let lines: Vec<_> = split_lines_preserve_trailing("a\r\nb\r\n").collect();
    assert_eq!(lines, vec!["a", "b", ""]);
}

#[test]
fn test_split_lines_mixed_endings() {
    use super::split_lines_preserve_trailing;

    // Mixed line endings
    let lines: Vec<_> = split_lines_preserve_trailing("a\r\nb\nc\r\n").collect();
    assert_eq!(lines, vec!["a", "b", "c", ""]);
}

#[test]
fn test_whitespace_trailing_newline_in_process_text() {
    let mut buffer = StreamingChunkBuffer::new();

    // Text with trailing newline - the empty trailing line should be preserved
    let text = "Line 1\nLine 2\n";
    let chunks = buffer.process_text(text);

    // Should have: Line 1, Line 2, and empty (trailing newline)
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].content, "Line 1");
    assert_eq!(chunks[1].content, "Line 2");
    assert_eq!(chunks[2].content, ""); // trailing newline preserved
}

#[test]
fn test_whitespace_multiple_trailing_newlines_in_process_text() {
    let mut buffer = StreamingChunkBuffer::new();

    // Text with multiple trailing newlines
    let text = "Line\n\n\n";
    let chunks = buffer.process_text(text);

    // Should have: Line, empty, empty, empty
    assert_eq!(chunks.len(), 4);
    assert_eq!(chunks[0].content, "Line");
    assert_eq!(chunks[1].content, "");
    assert_eq!(chunks[2].content, "");
    assert_eq!(chunks[3].content, "");
}

// ==========================================================================
// Prose buffer threshold tests (progressive streaming)
// ==========================================================================

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
