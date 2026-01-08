//! Whitespace preservation tests for StreamProcessor.
//!
//! Tests that blank lines, indentation, and whitespace are correctly
//! preserved in the output.

use crate::stream_processor::*;
use ralph_core::chunk::ChunkType;

#[test]
fn test_whitespace_blank_lines_preserved_between_paragraphs() {
    let mut processor = StreamProcessor::with_highlighting(false);

    // Simulate: "Paragraph 1.\n\nParagraph 2."
    let output1 = processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Paragraph 1.\n\nParagraph 2."}]}}"#,
    );

    let result = processor.finish();

    // Should have three chunks: Paragraph 1, blank line, Paragraph 2
    assert_eq!(result.chunks.len(), 3);
    assert_eq!(result.chunks[0].content, "Paragraph 1.");
    assert_eq!(result.chunks[1].content, ""); // blank line preserved
    assert_eq!(result.chunks[2].content, "Paragraph 2.");

    // raw_text should preserve the original
    assert_eq!(result.raw_text, "Paragraph 1.\n\nParagraph 2.");

    // Output should have correct newlines
    if let Some(out) = output1 {
        // Each chunk gets a newline, so: "Paragraph 1.\n" + "\n" + "Paragraph 2.\n"
        assert_eq!(out, "Paragraph 1.\n\nParagraph 2.\n");
    }
}

#[test]
fn test_whitespace_multiple_blank_lines_preserved() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Text\n\n\nMore text"}]}}"#,
    );

    let result = processor.finish();

    // Should have: Text, blank, blank, More text
    assert_eq!(result.chunks.len(), 4);
    assert_eq!(result.chunks[0].content, "Text");
    assert_eq!(result.chunks[1].content, "");
    assert_eq!(result.chunks[2].content, "");
    assert_eq!(result.chunks[3].content, "More text");
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

    // Text with trailing newline
    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Line 1\nLine 2\n"}]}}"#,
    );

    let result = processor.finish();

    // Should preserve trailing newline as empty chunk
    assert_eq!(result.chunks.len(), 3);
    assert_eq!(result.chunks[0].content, "Line 1");
    assert_eq!(result.chunks[1].content, "Line 2");
    assert_eq!(result.chunks[2].content, ""); // trailing newline
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

    assert_eq!(result.chunks.len(), 3);
    assert_eq!(result.chunks[0].content, "- Item 1");
    assert_eq!(result.chunks[1].content, "  - Nested item");
    assert_eq!(result.chunks[2].content, "    - Deeply nested");
}

#[test]
fn test_whitespace_blank_line_before_code_block() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"Here's code:\n\n```rust\nfn main() {}\n```"}]}}"#,
    );

    let result = processor.finish();

    // Should have: prose ("Here's code:"), blank line, code block
    assert_eq!(result.chunks.len(), 3);
    assert_eq!(result.chunks[0].content, "Here's code:");
    assert_eq!(result.chunks[1].content, ""); // blank line before code
    assert!(matches!(
        result.chunks[2].chunk_type,
        ChunkType::Code { .. }
    ));
}

#[test]
fn test_whitespace_blank_line_after_code_block() {
    let mut processor = StreamProcessor::with_highlighting(false);

    processor.process_line(
        r#"{"type":"assistant","message":{"id":"1","content":[{"type":"text","text":"```rust\nfn main() {}\n```\n\nDone."}]}}"#,
    );

    let result = processor.finish();

    // Should have: code block, blank line, prose ("Done.")
    assert_eq!(result.chunks.len(), 3);
    assert!(matches!(
        result.chunks[0].chunk_type,
        ChunkType::Code { .. }
    ));
    assert_eq!(result.chunks[1].content, ""); // blank line after code
    assert_eq!(result.chunks[2].content, "Done.");
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
