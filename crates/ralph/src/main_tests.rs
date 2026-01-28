//! Tests for main.rs functions.

use super::*;

#[test]
fn test_substitute_prompt_simple() {
    let result = substitute_prompt_in_command("echo {prompt}", "hello");
    assert_eq!(result, "echo 'hello'");
}

#[test]
fn test_substitute_prompt_with_quotes() {
    let result = substitute_prompt_in_command("echo {prompt}", "it's a test");
    assert_eq!(result, "echo 'it'\"'\"'s a test'");
}

#[test]
fn test_substitute_prompt_claude_command() {
    let result = substitute_prompt_in_command(
        "claude --permission-mode acceptEdits --output-format stream-json -p {prompt}",
        "test prompt",
    );
    assert_eq!(
        result,
        "claude --permission-mode acceptEdits --output-format stream-json -p 'test prompt'"
    );
}

#[test]
fn test_substitute_prompt_custom_command_structure() {
    // Custom commands can use any structure
    let result = substitute_prompt_in_command("my-llm --input {prompt} --verbose", "hello");
    assert_eq!(result, "my-llm --input 'hello' --verbose");
}

#[test]
fn test_substitute_prompt_custom_command_with_env_vars() {
    // Custom command with environment variables and complex structure
    let result = substitute_prompt_in_command("OPENAI_KEY=$KEY openai-cli chat {prompt}", "query");
    assert_eq!(result, "OPENAI_KEY=$KEY openai-cli chat 'query'");
}

#[test]
fn test_substitute_prompt_empty_prompt() {
    let result = substitute_prompt_in_command("echo {prompt}", "");
    assert_eq!(result, "echo ''");
}

#[test]
fn test_substitute_prompt_multiline() {
    let result = substitute_prompt_in_command("echo {prompt}", "line1\nline2");
    assert_eq!(result, "echo 'line1\nline2'");
}

#[test]
fn test_substitute_prompt_special_shell_chars() {
    // Shell special characters should be safely escaped within single quotes
    let result = substitute_prompt_in_command("echo {prompt}", "test $VAR `cmd` $(cmd)");
    assert_eq!(result, "echo 'test $VAR `cmd` $(cmd)'");
}

#[test]
fn test_substitute_prompt_no_placeholder() {
    // Command without {prompt} placeholder returns unchanged
    let result = substitute_prompt_in_command("echo hello", "ignored");
    assert_eq!(result, "echo hello");
}

#[test]
fn test_default_command_template_substitution() {
    // Verify the default command template works correctly
    let result = substitute_prompt_in_command(defaults::COMMAND_TEMPLATE, "my prompt");
    assert!(result.starts_with("claude "));
    assert!(result.contains("--output-format stream-json"));
    assert!(result.contains("'my prompt'"));
    // Session ID is NOT in default template - each iteration is a fresh session
    assert!(!result.contains("--session-id"));
}

#[test]
fn test_failure_recovery_context_creation() {
    let ctx = FailureRecoveryContext {
        summary: "Test failure".to_string(),
        session_slug: "test-session".to_string(),
        iterations_completed: 3,
        total_iterations_completed: 5,
    };

    assert_eq!(ctx.summary, "Test failure");
    assert_eq!(ctx.session_slug, "test-session");
    assert_eq!(ctx.iterations_completed, 3);
    assert_eq!(ctx.total_iterations_completed, 5);
}

#[test]
fn test_classify_prompt_source_stdin() {
    assert_eq!(classify_prompt_source(Some("-")), PromptSource::Stdin);
}

#[test]
fn test_classify_prompt_source_none() {
    assert_eq!(classify_prompt_source(None), PromptSource::None);
}

#[test]
fn test_classify_prompt_source_inline() {
    // Non-existent path should be treated as inline
    assert_eq!(
        classify_prompt_source(Some("inline content")),
        PromptSource::Inline("inline content")
    );
}

#[test]
fn test_classify_prompt_source_file() {
    // Use Cargo.toml as a file that definitely exists
    let source = classify_prompt_source(Some("Cargo.toml"));
    assert!(matches!(source, PromptSource::File(_)));
}

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
