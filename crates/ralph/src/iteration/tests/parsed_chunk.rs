//! Tests for ParsedChunk to Chunk conversion.

use super::super::*;
use ralph_core::chunk::ParsedChunk;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_chunk_from_parsed_chunk_prose() {
    let parsed = ParsedChunk::prose("Some prose content");
    let chunk = Chunk::from_parsed_chunk(&parsed);

    assert_eq!(chunk.chunk_type, "prose");
    assert_eq!(chunk.content, "Some prose content");
    assert!(chunk.language.is_none());
}

#[test]
fn test_chunk_from_parsed_chunk_code_with_language() {
    let parsed = ParsedChunk::code("fn main() {}", Some("rust".to_string()));
    let chunk = Chunk::from_parsed_chunk(&parsed);

    assert_eq!(chunk.chunk_type, "code");
    assert_eq!(chunk.content, "fn main() {}");
    assert_eq!(chunk.language, Some("rust".to_string()));
}

#[test]
fn test_chunk_from_parsed_chunk_code_without_language() {
    let parsed = ParsedChunk::code("some code", None);
    let chunk = Chunk::from_parsed_chunk(&parsed);

    assert_eq!(chunk.chunk_type, "code");
    assert_eq!(chunk.content, "some code");
    assert!(chunk.language.is_none());
}

#[test]
fn test_chunk_from_parsed_chunk_diff() {
    let parsed = ParsedChunk::diff("-old\n+new");
    let chunk = Chunk::from_parsed_chunk(&parsed);

    assert_eq!(chunk.chunk_type, "diff");
    assert_eq!(chunk.content, "-old\n+new");
    assert!(chunk.language.is_none());
}

#[test]
fn test_chunk_from_parsed_chunks_preserves_order() {
    let parsed_chunks = vec![
        ParsedChunk::prose("intro"),
        ParsedChunk::code("fn main() {}", Some("rust".to_string())),
        ParsedChunk::diff("-a\n+b"),
        ParsedChunk::prose("outro"),
    ];

    let chunks = Chunk::from_parsed_chunks(&parsed_chunks);

    assert_eq!(chunks.len(), 4);
    assert_eq!(chunks[0].chunk_type, "prose");
    assert_eq!(chunks[0].content, "intro");
    assert_eq!(chunks[1].chunk_type, "code");
    assert_eq!(chunks[1].language, Some("rust".to_string()));
    assert_eq!(chunks[2].chunk_type, "diff");
    assert_eq!(chunks[3].chunk_type, "prose");
    assert_eq!(chunks[3].content, "outro");
}

#[test]
fn test_chunk_from_parsed_chunks_empty() {
    let parsed_chunks: Vec<ParsedChunk> = vec![];
    let chunks = Chunk::from_parsed_chunks(&parsed_chunks);

    assert!(chunks.is_empty());
}

#[test]
fn test_chunk_from_parsed_chunks_serializes_to_toml() {
    let parsed_chunks = vec![
        ParsedChunk::prose("Here's some code:"),
        ParsedChunk::code("print('hello')", Some("python".to_string())),
    ];

    let chunks = Chunk::from_parsed_chunks(&parsed_chunks);

    // Create an iteration log with these chunks
    let now = chrono::Utc::now();
    let log = IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 5,
        pending_after: 4,
        prompt: None,
        response: None,
        metadata: None,
        tool_calls: vec![],
        chunks,
        output_blocks: vec![],
    };

    let toml_str = toml::to_string_pretty(&log).unwrap();

    // Verify TOML structure
    assert!(toml_str.contains("[[chunks]]"));
    assert!(toml_str.contains("type = \"prose\""));
    assert!(toml_str.contains("type = \"code\""));
    assert!(toml_str.contains("language = \"python\""));
    assert!(toml_str.contains("Here's some code:"));
    assert!(toml_str.contains("print('hello')"));

    // Verify round-trip
    let parsed: IterationLog = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.chunks.len(), 2);
    assert_eq!(parsed.chunks[0].chunk_type, "prose");
    assert_eq!(parsed.chunks[1].chunk_type, "code");
    assert_eq!(parsed.chunks[1].language, Some("python".to_string()));
}

#[test]
fn test_chunk_from_parsed_chunks_diff_serialization() {
    let parsed = ParsedChunk::diff("--- a/file.rs\n+++ b/file.rs\n@@ -1,1 +1,1 @@\n-old\n+new");
    let chunk = Chunk::from_parsed_chunk(&parsed);

    let toml_str = toml::to_string(&chunk).unwrap();

    // Diff chunks should have type = "diff" and no language field
    assert!(toml_str.contains("type = \"diff\""));
    assert!(!toml_str.contains("language"));
}

#[test]
fn test_iteration_log_with_typed_chunks_from_parsed() {
    let temp_dir = TempDir::new().unwrap();
    let session_dir = temp_dir.path();

    // Simulate chunks parsed from stream output
    let parsed_chunks = vec![
        ParsedChunk::prose("I'll implement this feature"),
        ParsedChunk::code(
            "fn authenticate(user: &str) -> bool {\n    true\n}",
            Some("rust".to_string()),
        ),
        ParsedChunk::prose("Here are the changes:"),
        ParsedChunk::diff("--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -1 +1,3 @@\n+fn authenticate(user: &str) -> bool {\n+    true\n+}"),
    ];

    let chunks = Chunk::from_parsed_chunks(&parsed_chunks);

    let now = chrono::Utc::now();
    let log = IterationLog {
        sequence: 1,
        started_at: now,
        completed_at: now,
        exit_code: 0,
        pending_before: 5,
        pending_after: 4,
        prompt: None,
        response: None,
        metadata: None,
        tool_calls: vec![],
        chunks,
        output_blocks: vec![],
    };

    // Write to disk
    let log_path = write_iteration_log(session_dir, &log).unwrap();

    // Read back and verify
    let content = fs::read_to_string(&log_path).unwrap();
    let parsed_log: IterationLog = toml::from_str(&content).unwrap();

    assert_eq!(parsed_log.chunks.len(), 4);
    assert_eq!(parsed_log.chunks[0].chunk_type, "prose");
    assert_eq!(parsed_log.chunks[1].chunk_type, "code");
    assert_eq!(parsed_log.chunks[1].language, Some("rust".to_string()));
    assert_eq!(parsed_log.chunks[2].chunk_type, "prose");
    assert_eq!(parsed_log.chunks[3].chunk_type, "diff");
    assert!(parsed_log.chunks[3].language.is_none());
}

#[test]
fn test_chunk_equality() {
    let chunk1 = Chunk::prose("hello".to_string());
    let chunk2 = Chunk::prose("hello".to_string());
    let chunk3 = Chunk::prose("world".to_string());

    assert_eq!(chunk1, chunk2);
    assert_ne!(chunk1, chunk3);

    let code1 = Chunk::code("fn main() {}".to_string(), Some("rust".to_string()));
    let code2 = Chunk::code("fn main() {}".to_string(), Some("rust".to_string()));
    let code3 = Chunk::code("fn main() {}".to_string(), Some("python".to_string()));

    assert_eq!(code1, code2);
    assert_ne!(code1, code3);
}
