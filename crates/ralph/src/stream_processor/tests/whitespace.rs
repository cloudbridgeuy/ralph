//! Whitespace preservation tests for StreamProcessor.
//!
//! Tests that blank lines, indentation, and whitespace are correctly
//! preserved in the output.
//!
//! With block buffering (prose_threshold = usize::MAX), all prose accumulates
//! into single multi-line chunks. Prose is only flushed when a code fence opens
//! or at finish() time.

use crate::stream_processor::*;
use ralph_core::chunk::ChunkType;

#[test]
fn test_whitespace_blank_lines_preserved_between_paragraphs() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // Simulate: "Paragraph 1.\n\nParagraph 2."
    // With block buffering, prose accumulates — process_line returns None
    let output = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Paragraph 1.\n\nParagraph 2."}]}}"#,
    );
    assert!(output.is_none(), "prose should be buffered, not emitted");

    let result = processor.finish();

    // Block buffering: all prose in a single chunk
    assert_eq!(result.chunks.len(), 1);
    assert_eq!(result.chunks[0].content, "Paragraph 1.\n\nParagraph 2.");
    assert!(matches!(result.chunks[0].chunk_type, ChunkType::Prose));

    // raw_text should preserve the original
    assert_eq!(result.raw_text, "Paragraph 1.\n\nParagraph 2.");

    // Final output should contain the rendered prose
    assert!(result.final_output.is_some());
    let final_out = result.final_output.as_ref().unwrap();
    assert!(final_out.contains("Paragraph 1."));
    assert!(final_out.contains("Paragraph 2."));
}

#[test]
fn test_whitespace_multiple_blank_lines_preserved() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Text\n\n\nMore text"}]}}"#,
    );

    let result = processor.finish();

    // Block buffering: single prose chunk preserving all blank lines
    assert_eq!(result.chunks.len(), 1);
    assert_eq!(result.chunks[0].content, "Text\n\n\nMore text");
}

#[test]
fn test_whitespace_code_block_content_preserved() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // Code with internal blank line
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```rust\nfn a() {}\n\nfn b() {}\n```"}]}}"#,
    );

    let result = processor.finish();

    // Find the code chunk
    let code_chunk = result
        .chunks
        .iter()
        .find(|c| matches!(c.chunk_type, ChunkType::Code { .. }))
        .expect("Should have code chunk");

    // Internal blank line should be preserved
    assert_eq!(code_chunk.content, "fn a() {}\n\nfn b() {}");
}

#[test]
fn test_whitespace_indentation_preserved_in_code() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```python\ndef foo():\n    x = 1\n        nested = 2\n```"}]}}"#,
    );

    let result = processor.finish();

    let code_chunk = result
        .chunks
        .iter()
        .find(|c| matches!(c.chunk_type, ChunkType::Code { .. }))
        .expect("Should have code chunk");

    // Indentation preserved exactly
    assert_eq!(
        code_chunk.content,
        "def foo():\n    x = 1\n        nested = 2"
    );
}

#[test]
fn test_whitespace_trailing_newline_in_text() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // Text with trailing newline: "Line 1\nLine 2\n"
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Line 1\nLine 2\n"}]}}"#,
    );

    let result = processor.finish();

    // Block buffering: single prose chunk preserving the trailing newline
    assert_eq!(result.chunks.len(), 1);
    assert_eq!(result.chunks[0].content, "Line 1\nLine 2\n");
}

#[test]
fn test_whitespace_leading_spaces_preserved() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"    indented line"}]}}"#,
    );

    let result = processor.finish();

    assert_eq!(result.chunks.len(), 1);
    assert_eq!(result.chunks[0].content, "    indented line");
}

#[test]
fn test_whitespace_list_indentation_preserved() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"- Item 1\n  - Nested item\n    - Deeply nested"}]}}"#,
    );

    let result = processor.finish();

    // Block buffering: single prose chunk with all list items
    assert_eq!(result.chunks.len(), 1);
    assert_eq!(
        result.chunks[0].content,
        "- Item 1\n  - Nested item\n    - Deeply nested"
    );
}

#[test]
fn test_whitespace_blank_line_before_code_block() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Here's code:\n\n```rust\nfn main() {}\n```"}]}}"#,
    );

    let result = processor.finish();

    // Should have 2 chunks: prose flushed when code fence opens, then code block
    assert_eq!(result.chunks.len(), 2);
    assert_eq!(result.chunks[0].content, "Here's code:\n");
    assert!(matches!(result.chunks[0].chunk_type, ChunkType::Prose));
    assert!(matches!(
        result.chunks[1].chunk_type,
        ChunkType::Code { .. }
    ));
    assert_eq!(result.chunks[1].content, "fn main() {}");
}

#[test]
fn test_whitespace_blank_line_after_code_block() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```rust\nfn main() {}\n```\n\nDone."}]}}"#,
    );

    let result = processor.finish();

    // Should have 2 chunks: code block (immediate), then prose from final flush
    assert_eq!(result.chunks.len(), 2);
    assert!(matches!(
        result.chunks[0].chunk_type,
        ChunkType::Code { .. }
    ));
    // The blank line after ``` produces an empty prose line, which joins with "Done."
    // Since accumulate_prose_line("") leaves the buffer empty (len 0), the next line
    // doesn't get a \n prefix, resulting in just "Done."
    assert_eq!(result.chunks[1].content, "Done.");
    assert!(matches!(result.chunks[1].chunk_type, ChunkType::Prose));
}

#[test]
fn test_whitespace_raw_text_matches_original() {
    let mut processor = StreamProcessor::with_highlighting(false);

    let original = "Hello\n\nWorld\n\n```rust\ncode\n```\n\nDone";
    processor.process_line(&format!(
        r#"{{"type":"assistant","message":{{"id":"1","content":[{{"type":"text","text":"{}"}}]}}}}"#,
        original.replace('\n', "\\n")
    ));

    let result = processor.finish();

    // raw_text should match original exactly
    assert_eq!(result.raw_text, original);
}

#[test]
fn test_whitespace_across_multiple_events() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // First event ends mid-paragraph
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Hello "}]}}"#,
    );

    // Second event continues
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"World\n\nNext paragraph"}]}}"#,
    );

    let result = processor.finish();

    // raw_text should be "Hello World\n\nNext paragraph"
    assert_eq!(result.raw_text, "Hello World\n\nNext paragraph");
}
