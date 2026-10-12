//! Tests for startup display functionality.

use std::path::PathBuf;

use super::formatters::{format_duration, format_token_count};
use super::types::{clean_blank_lines, AskSummary, AttachedFile, PromptDisplay};

// Tests for format_duration

#[test]
fn test_format_duration_milliseconds() {
    assert_eq!(format_duration(0), "0ms");
    assert_eq!(format_duration(500), "500ms");
    assert_eq!(format_duration(999), "999ms");
}

#[test]
fn test_format_duration_seconds() {
    assert_eq!(format_duration(1000), "1.0s");
    assert_eq!(format_duration(1500), "1.5s");
    assert_eq!(format_duration(45200), "45.2s");
    assert_eq!(format_duration(59999), "60.0s");
}

#[test]
fn test_format_duration_minutes() {
    assert_eq!(format_duration(60_000), "1m 0s");
    assert_eq!(format_duration(83_000), "1m 23s");
    assert_eq!(format_duration(120_000), "2m 0s");
    assert_eq!(format_duration(600_000), "10m 0s");
}

#[test]
fn test_format_duration_hours_as_minutes() {
    // Very long durations are shown as minutes
    assert_eq!(format_duration(3_600_000), "60m 0s");
    assert_eq!(format_duration(7_200_000), "120m 0s");
}

// Tests for format_token_count

#[test]
fn test_format_token_count_small() {
    assert_eq!(format_token_count(0), "0");
    assert_eq!(format_token_count(100), "100");
    assert_eq!(format_token_count(999), "999");
}

#[test]
fn test_format_token_count_thousands() {
    assert_eq!(format_token_count(1000), "1.0K");
    assert_eq!(format_token_count(1500), "1.5K");
    assert_eq!(format_token_count(45_200), "45.2K");
    assert_eq!(format_token_count(999_999), "1000.0K");
}

#[test]
fn test_format_token_count_millions() {
    assert_eq!(format_token_count(1_000_000), "1.00M");
    assert_eq!(format_token_count(1_500_000), "1.50M");
    assert_eq!(format_token_count(10_000_000), "10.00M");
}

// Tests for AttachedFile

#[test]
fn test_attached_file_design_md_not_special_cased() {
    let file = AttachedFile::new(PathBuf::from("/project/.local/designs/design.md"));
    assert_eq!(file.description, "Attached file");
}

#[test]
fn test_attached_file_progress_txt_not_special_cased() {
    let file = AttachedFile::new(PathBuf::from("/project/.local/plans/progress.txt"));
    assert_eq!(file.description, "Attached file");
}

#[test]
fn test_attached_file_unknown() {
    let file = AttachedFile::new(PathBuf::from("/project/some/other/file.rs"));
    assert_eq!(file.description, "Attached file");
}

#[test]
fn test_attached_file_path_preserved() {
    let path = PathBuf::from("/my/custom/path/design.md");
    let file = AttachedFile::new(path.clone());
    assert_eq!(file.path, path);
}

// Tests for PromptDisplay

#[test]
fn test_prompt_display_from_prompt_extracts_files() {
    let prompt = "@/project/design.md @/project/spec.toml @/project/progress.txt

1. Do something
2. Do another thing";

    let display = PromptDisplay::from_prompt(prompt);
    assert_eq!(display.attached_files.len(), 3);
    assert_eq!(
        display.attached_files[0].path,
        PathBuf::from("/project/design.md")
    );
    assert_eq!(
        display.attached_files[1].path,
        PathBuf::from("/project/spec.toml")
    );
    assert_eq!(
        display.attached_files[2].path,
        PathBuf::from("/project/progress.txt")
    );
}

#[test]
fn test_prompt_display_from_prompt_no_files() {
    let prompt = "Just a prompt with no file references";
    let display = PromptDisplay::from_prompt(prompt);
    assert!(display.attached_files.is_empty());
}

#[test]
fn test_prompt_display_from_prompt_preserves_prompt() {
    let prompt = "@/file.txt\nSome content";
    let display = PromptDisplay::from_prompt(prompt);
    assert_eq!(display.prompt, prompt);
}

#[test]
fn test_prompt_display_from_prompt_handles_at_symbol_in_text() {
    // @ followed by non-path characters should not be treated as file reference
    let prompt = "Email me at john@example.com";
    let display = PromptDisplay::from_prompt(prompt);
    // "john" starts with a letter, so it will be captured but that's expected
    // The key is that we handle the text without crashing
    assert_eq!(display.prompt, prompt);
}

#[test]
fn test_prompt_display_from_prompt_relative_path() {
    let prompt = "@./local/file.txt @file.rs";
    let display = PromptDisplay::from_prompt(prompt);
    // Relative paths are supported (no leading /)
    assert!(!display.attached_files.is_empty());
}

#[test]
fn test_prompt_display_stripped_prompt_removes_references() {
    let prompt = "@/project/design.md @/project/spec.toml

1. Do something
2. Do another thing";

    let display = PromptDisplay::from_prompt(prompt);
    let stripped = display.stripped_prompt();

    // Should not contain file references
    assert!(!stripped.contains("@/project/design.md"));
    assert!(!stripped.contains("@/project/spec.toml"));
    // Should contain the actual content
    assert!(stripped.contains("1. Do something"));
    assert!(stripped.contains("2. Do another thing"));
}

#[test]
fn test_prompt_display_stripped_prompt_preserves_content() {
    let prompt = "No file references here, just content";
    let display = PromptDisplay::from_prompt(prompt);
    let stripped = display.stripped_prompt();
    assert_eq!(stripped, prompt);
}

// Tests for clean_blank_lines

#[test]
fn test_clean_blank_lines_collapses_multiple() {
    let input = "line1\n\n\n\nline2";
    let result = clean_blank_lines(input);
    assert_eq!(result, "line1\n\nline2");
}

#[test]
fn test_clean_blank_lines_preserves_single() {
    let input = "line1\n\nline2";
    let result = clean_blank_lines(input);
    assert_eq!(result, "line1\n\nline2");
}

#[test]
fn test_clean_blank_lines_preserves_content_lines() {
    let input = "line1\nline2\nline3";
    let result = clean_blank_lines(input);
    assert_eq!(result, "line1\nline2\nline3");
}

#[test]
fn test_clean_blank_lines_empty_input() {
    let input = "";
    let result = clean_blank_lines(input);
    assert_eq!(result, "");
}

#[test]
fn test_clean_blank_lines_only_blanks() {
    let input = "\n\n\n";
    let result = clean_blank_lines(input);
    // Multiple blank lines collapse to single newline
    assert_eq!(result, "\n");
}

// Tests for AskSummary

fn create_test_ask_summary() -> AskSummary {
    AskSummary {
        success: true,
        cost_usd: Some(0.05),
        duration_ms: Some(10_000),
        input_tokens: Some(500),
        output_tokens: Some(1500),
    }
}

#[test]
fn test_ask_summary_creation() {
    let summary = create_test_ask_summary();
    assert!(summary.success);
    assert_eq!(summary.cost_usd, Some(0.05));
    assert_eq!(summary.duration_ms, Some(10_000));
    assert_eq!(summary.input_tokens, Some(500));
    assert_eq!(summary.output_tokens, Some(1500));
}

#[test]
fn test_ask_summary_failed() {
    let summary = AskSummary {
        success: false,
        cost_usd: Some(0.01),
        duration_ms: Some(5_000),
        input_tokens: Some(100),
        output_tokens: None,
    };
    assert!(!summary.success);
    assert_eq!(summary.cost_usd, Some(0.01));
}

#[test]
fn test_ask_summary_with_none_values() {
    let summary = AskSummary {
        success: true,
        cost_usd: None,
        duration_ms: None,
        input_tokens: None,
        output_tokens: None,
    };
    assert!(summary.success);
    assert!(summary.cost_usd.is_none());
    assert!(summary.duration_ms.is_none());
    assert!(summary.input_tokens.is_none());
    assert!(summary.output_tokens.is_none());
}

#[test]
fn test_ask_summary_clone() {
    let original = create_test_ask_summary();
    let cloned = original.clone();
    assert_eq!(original.success, cloned.success);
    assert_eq!(original.cost_usd, cloned.cost_usd);
    assert_eq!(original.duration_ms, cloned.duration_ms);
}

#[test]
fn test_ask_summary_zero_cost() {
    // Zero cost is valid (e.g., cached responses)
    let summary = AskSummary {
        success: true,
        cost_usd: Some(0.0),
        duration_ms: Some(100),
        input_tokens: Some(0),
        output_tokens: Some(0),
    };
    assert_eq!(summary.cost_usd, Some(0.0));
    assert_eq!(summary.input_tokens, Some(0));
    assert_eq!(summary.output_tokens, Some(0));
}

#[test]
fn test_ask_summary_large_values() {
    // Large token counts and costs
    let summary = AskSummary {
        success: true,
        cost_usd: Some(5.5678),
        duration_ms: Some(300_000), // 5 minutes
        input_tokens: Some(100_000),
        output_tokens: Some(50_000),
    };
    assert_eq!(summary.cost_usd, Some(5.5678));
    assert_eq!(summary.input_tokens, Some(100_000));
}
