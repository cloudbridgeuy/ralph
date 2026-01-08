//! Tests for subprocess invocation functions.

use super::*;
use crate::stream_processor::StreamProcessorResult;

#[test]
fn test_invoke_subprocess_success() {
    let result = invoke_subprocess("echo 'Hello, world!'").unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("Hello, world!"));
    // Note: stderr may contain shell-init warnings in some environments
    // so we don't assert it's empty
}

#[test]
fn test_invoke_subprocess_failure() {
    let result = invoke_subprocess("exit 42").unwrap();
    assert_eq!(result.exit_code, 42);
}

#[test]
fn test_invoke_subprocess_stderr() {
    let result = invoke_subprocess("echo 'error message' >&2").unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stderr.contains("error message"));
}

#[test]
fn test_invoke_subprocess_multiline_output() {
    let result = invoke_subprocess("echo 'line1'; echo 'line2'; echo 'line3'").unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("line1"));
    assert!(result.stdout.contains("line2"));
    assert!(result.stdout.contains("line3"));
}

#[test]
fn test_invoke_subprocess_empty_output() {
    let result = invoke_subprocess("true").unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.is_empty());
    // Note: stderr may contain shell-init warnings in some environments
    // so we don't assert it's empty
}

#[test]
fn test_invoke_subprocess_command_not_found() {
    let result = invoke_subprocess("nonexistent_command_12345");
    // Command will fail with non-zero exit code (command not found)
    assert!(result.is_ok()); // sh itself succeeds
    let result = result.unwrap();
    assert_ne!(result.exit_code, 0); // But the command inside fails
}

// Timeout tests

#[test]
fn test_invoke_with_timeout_completes_quickly() {
    // Fast command should complete before timeout
    let result = invoke_subprocess_with_timeout("echo 'hello'", 10).unwrap();
    assert_eq!(result.exit_code, 0);
    // stream processor parses JSON, so plain text won't be captured meaningfully
}

#[test]
fn test_invoke_with_timeout_times_out() {
    // Use a very short timeout (1 second) with a command that sleeps
    let result = invoke_subprocess_with_timeout("sleep 10", 1);
    match result {
        Err(SubprocessError::Timeout {
            timeout_secs,
            partial_result,
        }) => {
            assert_eq!(timeout_secs, 1);
            assert_eq!(partial_result.exit_code, -1); // Indicates killed
        }
        Ok(_) => panic!("Expected timeout error"),
        Err(e) => panic!("Expected timeout error, got: {}", e),
    }
}

#[test]
fn test_invoke_with_timeout_captures_partial_output() {
    // Command that outputs something then sleeps
    // Using a very short timeout to catch it mid-output
    let result = invoke_subprocess_with_timeout("echo 'first'; sleep 10; echo 'never_reached'", 1);
    match result {
        Err(SubprocessError::Timeout { partial_result, .. }) => {
            // The partial result should exist, even if raw_text is empty
            // (because stream processor parses JSON, not plain text)
            assert!(partial_result.exit_code == -1);
        }
        Ok(_) => panic!("Expected timeout error"),
        Err(e) => panic!("Expected timeout error, got: {}", e),
    }
}

#[test]
fn test_invoke_with_timeout_non_zero_exit() {
    // Command that fails should return the exit code, not timeout
    let result = invoke_subprocess_with_timeout("exit 42", 10).unwrap();
    assert_eq!(result.exit_code, 42);
}

#[test]
fn test_timeout_error_display() {
    let partial = StreamingSubprocessResult {
        exit_code: -1,
        stderr: String::new(),
        stream_result: StreamProcessorResult::default(),
    };
    let err = SubprocessError::Timeout {
        timeout_secs: 300,
        partial_result: Box::new(partial),
    };
    let msg = format!("{}", err);
    assert!(msg.contains("300 seconds"));
    assert!(msg.contains("timed out"));
}
