//! Tests for main.rs functions.

use super::*;

// Tests for resolve_ask_prompt

#[test]
fn test_resolve_ask_prompt_inline() {
    // Inline prompt should work
    let result = resolve_ask_prompt(Some("hello world"));
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "hello world");
}

#[test]
fn test_resolve_ask_prompt_inline_with_whitespace() {
    // Whitespace is preserved in result but validated as non-empty
    let result = resolve_ask_prompt(Some("  hello world  "));
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "  hello world  ");
}

#[test]
fn test_resolve_ask_prompt_empty_string_error() {
    // Empty string should error
    let result = resolve_ask_prompt(Some(""));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("cannot be empty"));
}

#[test]
fn test_resolve_ask_prompt_whitespace_only_error() {
    // Whitespace-only should error
    let result = resolve_ask_prompt(Some("   \n\t  "));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("cannot be empty"));
}

#[test]
fn test_resolve_ask_prompt_from_file() {
    // Read from existing file (Cargo.toml has content)
    let result = resolve_ask_prompt(Some("Cargo.toml"));
    assert!(result.is_ok());
    let content = result.unwrap();
    assert!(content.contains("[package]")); // Cargo.toml starts with [package]
}

// Note: Testing stdin behavior (None argument, "-" argument) is difficult in unit tests
// because stdin detection with is_terminal() returns false in test runners, making
// the behavior unpredictable. Integration tests with PTY would be needed.
