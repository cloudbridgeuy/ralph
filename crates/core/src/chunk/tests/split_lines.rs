//! Tests for split_lines_preserve_trailing utility.

use crate::chunk::split_lines_preserve_trailing;

#[test]
fn test_split_lines_basic() {
    let lines: Vec<_> = split_lines_preserve_trailing("a\nb").collect();
    assert_eq!(lines, vec!["a", "b"]);
}

#[test]
fn test_split_lines_trailing_newline() {
    // With trailing newline - should include trailing empty string
    let lines: Vec<_> = split_lines_preserve_trailing("a\nb\n").collect();
    assert_eq!(lines, vec!["a", "b", ""]);
}

#[test]
fn test_split_lines_multiple_trailing_newlines() {
    // Multiple trailing newlines
    let lines: Vec<_> = split_lines_preserve_trailing("a\n\n").collect();
    assert_eq!(lines, vec!["a", "", ""]);
}

#[test]
fn test_split_lines_empty_string() {
    let lines: Vec<_> = split_lines_preserve_trailing("").collect();
    assert!(lines.is_empty());
}

#[test]
fn test_split_lines_single_newline() {
    // Just a newline
    let lines: Vec<_> = split_lines_preserve_trailing("\n").collect();
    assert_eq!(lines, vec!["", ""]);
}

#[test]
fn test_split_lines_no_newline() {
    let lines: Vec<_> = split_lines_preserve_trailing("hello").collect();
    assert_eq!(lines, vec!["hello"]);
}

#[test]
fn test_split_lines_blank_lines_middle() {
    // Blank lines in the middle
    let lines: Vec<_> = split_lines_preserve_trailing("a\n\nb").collect();
    assert_eq!(lines, vec!["a", "", "b"]);
}

#[test]
fn test_split_lines_crlf() {
    // CRLF line endings (Windows)
    let lines: Vec<_> = split_lines_preserve_trailing("a\r\nb\r\n").collect();
    assert_eq!(lines, vec!["a", "b", ""]);
}

#[test]
fn test_split_lines_mixed_endings() {
    // Mixed line endings
    let lines: Vec<_> = split_lines_preserve_trailing("a\r\nb\nc\r\n").collect();
    assert_eq!(lines, vec!["a", "b", "c", ""]);
}
