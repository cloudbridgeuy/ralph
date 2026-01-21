//! Tests for batch chunk parsing.

use crate::chunk::{
    is_unfenced_diff, parse_chunks, parse_chunks_with_heuristics, ChunkType, ParsedChunk,
};

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
    let json = serde_json::to_string(&prose).expect("serialize prose");
    assert!(json.contains(r#""type":"prose""#));

    // Test code serialization with language
    let code = ParsedChunk::code("fn main()", Some("rust".to_string()));
    let json = serde_json::to_string(&code).expect("serialize code");
    assert!(json.contains(r#""type":"code""#));
    assert!(json.contains(r#""language":"rust""#));

    // Test code serialization without language
    let code_no_lang = ParsedChunk::code("fn main()", None);
    let json = serde_json::to_string(&code_no_lang).expect("serialize code no lang");
    assert!(json.contains(r#""type":"code""#));
    assert!(!json.contains("language"));

    // Test diff serialization
    let diff = ParsedChunk::diff("-old\n+new");
    let json = serde_json::to_string(&diff).expect("serialize diff");
    assert!(json.contains(r#""type":"diff""#));
}

#[test]
fn test_chunk_type_deserialization() {
    let json = r#"{"type":"code","language":"python","content":"print(1)"}"#;
    let chunk: ParsedChunk = serde_json::from_str(json).expect("deserialize chunk");

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

    let json = serde_json::to_string(&original).expect("serialize");
    let deserialized: Vec<ParsedChunk> = serde_json::from_str(&json).expect("deserialize");

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

// =============================================================================
// Helper function tests
// =============================================================================

mod helpers {
    use crate::chunk::{parse_chunks, ChunkType};

    // We test the helpers indirectly through parse_chunks behavior
    // since they are private functions

    #[test]
    fn flush_prose_preserves_content() {
        // Verified through existing test_parse_simple_prose
        let chunks = parse_chunks("Hello\nWorld");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "Hello\nWorld");
    }

    #[test]
    fn append_line_joins_with_newline() {
        // Verified through multi-line prose handling
        let chunks = parse_chunks("Line1\nLine2\nLine3");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "Line1\nLine2\nLine3");
    }

    #[test]
    fn emit_code_block_handles_diff() {
        let chunks = parse_chunks("```diff\n-old\n+new\n```");
        assert_eq!(chunks.len(), 1);
        assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
    }

    #[test]
    fn emit_code_block_handles_language() {
        let chunks = parse_chunks("```typescript\nconst x = 1;\n```");
        assert_eq!(chunks.len(), 1);
        match &chunks[0].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("typescript"));
            }
            _ => panic!("Expected code chunk"),
        }
    }
}
