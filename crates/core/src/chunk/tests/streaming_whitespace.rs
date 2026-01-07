//! Tests for whitespace preservation in streaming.

use crate::chunk::{ChunkType, StreamingChunkBuffer};

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
