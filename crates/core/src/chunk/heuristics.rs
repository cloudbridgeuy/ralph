//! Diff detection heuristics for unfenced content.

use super::batch::parse_chunks;
use super::types::{ChunkType, ParsedChunk};

/// Minimum number of diff-like lines required to classify text as an unfenced diff.
/// Lines starting with `+` or `-` (excluding `++`/`--` headers) are counted.
const MIN_DIFF_LINES_THRESHOLD: usize = 2;

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

    diff_line_count >= MIN_DIFF_LINES_THRESHOLD
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
