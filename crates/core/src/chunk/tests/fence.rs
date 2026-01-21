//! Tests for fence detection utilities.

use crate::chunk::fence::{is_fence_close, parse_fence_open};

// =============================================================================
// parse_fence_open tests
// =============================================================================

#[test]
fn parse_fence_open_with_language() {
    let result = parse_fence_open("```rust");
    assert_eq!(result, Some(Some("rust".to_string())));
}

#[test]
fn parse_fence_open_with_language_and_whitespace() {
    let result = parse_fence_open("```python  ");
    assert_eq!(result, Some(Some("python".to_string())));
}

#[test]
fn parse_fence_open_bare_fence() {
    let result = parse_fence_open("```");
    assert_eq!(result, Some(None));
}

#[test]
fn parse_fence_open_bare_fence_with_trailing_whitespace() {
    let result = parse_fence_open("```   ");
    assert_eq!(result, Some(None));
}

#[test]
fn parse_fence_open_with_leading_whitespace() {
    let result = parse_fence_open("   ```javascript");
    assert_eq!(result, Some(Some("javascript".to_string())));
}

#[test]
fn parse_fence_open_not_a_fence() {
    let result = parse_fence_open("regular text");
    assert_eq!(result, None);
}

#[test]
fn parse_fence_open_partial_fence() {
    let result = parse_fence_open("``");
    assert_eq!(result, None);
}

#[test]
fn parse_fence_open_fence_in_text() {
    // Fence must be at the start (after optional whitespace)
    let result = parse_fence_open("text ```rust");
    assert_eq!(result, None);
}

#[test]
fn parse_fence_open_diff_language() {
    let result = parse_fence_open("```diff");
    assert_eq!(result, Some(Some("diff".to_string())));
}

#[test]
fn parse_fence_open_language_with_extra_words() {
    // Only first word is captured as language
    let result = parse_fence_open("```rust fn main");
    assert_eq!(result, Some(Some("rust".to_string())));
}

// =============================================================================
// is_fence_close tests
// =============================================================================

#[test]
fn is_fence_close_simple() {
    assert!(is_fence_close("```"));
}

#[test]
fn is_fence_close_with_surrounding_whitespace() {
    assert!(is_fence_close("  ```  "));
}

#[test]
fn is_fence_close_not_a_close() {
    assert!(!is_fence_close("```rust"));
}

#[test]
fn is_fence_close_partial() {
    assert!(!is_fence_close("``"));
}

#[test]
fn is_fence_close_empty_line() {
    assert!(!is_fence_close(""));
}

#[test]
fn is_fence_close_text_after_fence() {
    // Close fence must be exactly ``` (trimmed)
    assert!(!is_fence_close("``` text"));
}

#[test]
fn is_fence_close_fence_in_text() {
    assert!(!is_fence_close("text ```"));
}
