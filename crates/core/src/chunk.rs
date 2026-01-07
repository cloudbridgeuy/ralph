//! Chunk parsing for LLM output (Functional Core).
//!
//! This module provides pure functions for parsing accumulated text from
//! Claude's assistant events into typed chunks: prose (markdown), code
//! (fenced code blocks), and diff (unified diff format).
//!
//! # Chunk Types
//!
//! - **Prose**: Regular markdown text between code/diff blocks
//! - **Code**: Fenced code blocks (```) with optional language hint
//! - **Diff**: Unified diff format (```diff fence or heuristic detection)
//!
//! # Example
//!
//! ```
//! use ralph_core::chunk::{parse_chunks, ChunkType};
//!
//! let text = r#"I'll implement the function:
//!
//! ```rust
//! fn hello() {
//!     println!("Hello, world!");
//! }
//! ```
//!
//! This prints a greeting."#;
//!
//! let chunks = parse_chunks(text);
//! assert_eq!(chunks.len(), 3);
//! assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
//! assert!(matches!(chunks[1].chunk_type, ChunkType::Code { .. }));
//! assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
//! ```

use serde::{Deserialize, Serialize};

/// The type of a parsed chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ChunkType {
    /// Regular prose/markdown content.
    Prose,
    /// A fenced code block with optional language hint.
    Code {
        /// The language hint from the opening fence (e.g., "rust", "python").
        /// `None` if no language was specified.
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    /// A diff block (unified diff format).
    Diff,
}

/// A parsed chunk of LLM output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedChunk {
    /// The type of chunk (prose, code, or diff).
    #[serde(flatten)]
    pub chunk_type: ChunkType,
    /// The content of the chunk.
    pub content: String,
}

impl ParsedChunk {
    /// Create a prose chunk.
    pub fn prose(content: impl Into<String>) -> Self {
        Self {
            chunk_type: ChunkType::Prose,
            content: content.into(),
        }
    }

    /// Create a code chunk with optional language hint.
    pub fn code(content: impl Into<String>, language: Option<String>) -> Self {
        Self {
            chunk_type: ChunkType::Code { language },
            content: content.into(),
        }
    }

    /// Create a diff chunk.
    pub fn diff(content: impl Into<String>) -> Self {
        Self {
            chunk_type: ChunkType::Diff,
            content: content.into(),
        }
    }
}

/// Parse text into typed chunks.
///
/// This function scans text for fenced code blocks and diff patterns,
/// extracting them as typed chunks while preserving prose content between them.
///
/// # Arguments
///
/// * `text` - The accumulated text from assistant events
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
///
/// # Example
///
/// ```
/// use ralph_core::chunk::{parse_chunks, ChunkType};
///
/// let text = r#"Here's a simple function:
///
/// ```python
/// def greet():
///     print("Hello!")
/// ```
///
/// And here's a diff:
///
/// ```diff
/// -old line
/// +new line
/// ```
/// "#;
///
/// let chunks = parse_chunks(text);
/// assert_eq!(chunks.len(), 5);
/// ```
pub fn parse_chunks(text: &str) -> Vec<ParsedChunk> {
    let mut chunks = Vec::new();
    let mut current_prose = String::new();
    let mut in_code_block = false;
    let mut code_block_content = String::new();
    let mut code_block_language: Option<String> = None;
    let mut is_diff_block = false;

    for line in text.lines() {
        if !in_code_block {
            // Check for opening fence
            if let Some(lang) = parse_fence_open(line) {
                // Flush any accumulated prose
                if !current_prose.is_empty() {
                    chunks.push(ParsedChunk::prose(std::mem::take(&mut current_prose)));
                }

                in_code_block = true;
                is_diff_block = lang.as_deref() == Some("diff");
                code_block_language = lang;
                code_block_content.clear();
            } else {
                // Accumulate prose
                if !current_prose.is_empty() {
                    current_prose.push('\n');
                }
                current_prose.push_str(line);
            }
        } else {
            // Inside a code block
            if is_fence_close(line) {
                // End of code block
                if is_diff_block {
                    chunks.push(ParsedChunk::diff(std::mem::take(&mut code_block_content)));
                } else {
                    chunks.push(ParsedChunk::code(
                        std::mem::take(&mut code_block_content),
                        code_block_language.take(),
                    ));
                }
                in_code_block = false;
                is_diff_block = false;
            } else {
                // Accumulate code content
                if !code_block_content.is_empty() {
                    code_block_content.push('\n');
                }
                code_block_content.push_str(line);
            }
        }
    }

    // Handle unterminated code block
    if in_code_block && !code_block_content.is_empty() {
        if is_diff_block {
            chunks.push(ParsedChunk::diff(code_block_content));
        } else {
            chunks.push(ParsedChunk::code(code_block_content, code_block_language));
        }
    }

    // Flush remaining prose
    if !current_prose.is_empty() {
        chunks.push(ParsedChunk::prose(current_prose));
    }

    chunks
}

/// Check if a line is an opening fence and extract the language hint.
///
/// Returns `Some(Some(lang))` if a language is specified, `Some(None)` if it's
/// a bare fence, or `None` if it's not a fence at all.
fn parse_fence_open(line: &str) -> Option<Option<String>> {
    let trimmed = line.trim_start();

    // Check for triple backticks
    if let Some(after_fence) = trimmed.strip_prefix("```") {
        let after_fence = after_fence.trim();
        if after_fence.is_empty() {
            Some(None)
        } else {
            // Extract language (first word)
            let lang = after_fence.split_whitespace().next().unwrap_or("");
            Some(Some(lang.to_string()))
        }
    } else {
        None
    }
}

/// Check if a line is a closing fence.
fn is_fence_close(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed == "```"
}

/// Detect if text contains unfenced diff content using heuristics.
///
/// This function checks for common unified diff patterns like:
/// - Lines starting with `diff --git`
/// - Lines starting with `@@`
/// - Lines starting with `+` or `-` (excluding `++`/`--` headers)
///
/// # Arguments
///
/// * `text` - The text to check for diff patterns
///
/// # Returns
///
/// `true` if the text appears to contain unfenced diff content.
///
/// # Example
///
/// ```
/// use ralph_core::chunk::is_unfenced_diff;
///
/// let diff = r#"diff --git a/file.rs b/file.rs
/// --- a/file.rs
/// +++ b/file.rs
/// @@ -1,3 +1,4 @@
///  fn main() {
/// +    println!("Hello");
///  }
/// "#;
///
/// assert!(is_unfenced_diff(diff));
/// assert!(!is_unfenced_diff("Regular text without diffs"));
/// ```
pub fn is_unfenced_diff(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();

    // Check for explicit diff header
    if lines.iter().any(|l| l.starts_with("diff --git")) {
        return true;
    }

    // Check for hunk headers
    if lines
        .iter()
        .any(|l| l.starts_with("@@") && l.contains("@@"))
    {
        return true;
    }

    // Heuristic: multiple lines starting with + or - (but not ++ or --)
    let diff_line_count = lines
        .iter()
        .filter(|l| {
            (l.starts_with('+') && !l.starts_with("++"))
                || (l.starts_with('-') && !l.starts_with("--"))
        })
        .count();

    diff_line_count >= 2
}

/// Parse text that may contain unfenced diffs using heuristics.
///
/// This function first tries to parse using fenced code blocks. If no
/// code blocks are found and the text looks like a diff (using heuristics),
/// it treats the entire text as a diff chunk.
///
/// # Arguments
///
/// * `text` - The text to parse
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing the parsed content.
///
/// # Example
///
/// ```
/// use ralph_core::chunk::{parse_chunks_with_heuristics, ChunkType};
///
/// // Unfenced diff content
/// let text = r#"diff --git a/file.rs b/file.rs
/// --- a/file.rs
/// +++ b/file.rs
/// @@ -1,3 +1,4 @@
///  fn main() {
/// +    println!("Hello");
///  }
/// "#;
///
/// let chunks = parse_chunks_with_heuristics(text);
/// assert_eq!(chunks.len(), 1);
/// assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
/// ```
pub fn parse_chunks_with_heuristics(text: &str) -> Vec<ParsedChunk> {
    let chunks = parse_chunks(text);

    // If we only got prose chunks and the content looks like a diff, treat it as a diff
    if chunks.len() == 1 {
        if let ChunkType::Prose = &chunks[0].chunk_type {
            if is_unfenced_diff(&chunks[0].content) {
                return vec![ParsedChunk::diff(chunks[0].content.clone())];
            }
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
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
        let text =
            "```rust\nfn main() {\n    println!(\"Hello\");\n    println!(\"World\");\n}\n```";
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
}
