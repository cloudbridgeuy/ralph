//! Tests for Chunk type creation and serialization.

use super::super::*;

#[test]
fn test_chunk_prose() {
    let chunk = Chunk::prose("Hello, world!".to_string());
    assert_eq!(chunk.chunk_type, "prose");
    assert_eq!(chunk.content, "Hello, world!");
    assert!(chunk.language.is_none());
}

#[test]
fn test_chunk_code() {
    let chunk = Chunk::code("fn main() {}".to_string(), Some("rust".to_string()));
    assert_eq!(chunk.chunk_type, "code");
    assert_eq!(chunk.content, "fn main() {}");
    assert_eq!(chunk.language, Some("rust".to_string()));
}

#[test]
fn test_chunk_code_without_language() {
    let chunk = Chunk::code("some code".to_string(), None);
    assert_eq!(chunk.chunk_type, "code");
    assert_eq!(chunk.content, "some code");
    assert_eq!(chunk.language, None);
}

#[test]
fn test_chunk_diff() {
    let chunk = Chunk::diff("@@ -1,3 +1,3 @@".to_string());
    assert_eq!(chunk.chunk_type, "diff");
    assert_eq!(chunk.content, "@@ -1,3 +1,3 @@");
    assert!(chunk.language.is_none());
}

#[test]
fn test_chunk_serialization_omits_none_language() {
    let chunk = Chunk::prose("Plain text".to_string());

    #[derive(serde::Serialize)]
    struct Wrapper {
        chunk: Chunk,
    }

    let wrapper = Wrapper { chunk };
    let toml_str = toml::to_string(&wrapper).unwrap();

    // Language field should not appear when None
    assert!(!toml_str.contains("language"));
}
