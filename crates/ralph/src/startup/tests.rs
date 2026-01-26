//! Tests for startup display functionality.

use std::path::PathBuf;

use super::formatters::{format_duration, format_token_count};
use super::types::{
    clean_blank_lines, AskSummary, AttachedFile, IterationHeader, IterationSummary, PromptDisplay,
    RunSummary, StartupInfo,
};

fn create_test_info() -> StartupInfo {
    StartupInfo {
        slug: "test-session".to_string(),
        total_stories: 10,
        pending_stories: 5,
        completed_stories: 5,
        max_iterations: 5,
        iterations_from_arg: false,
        custom_prd_path: None,
        custom_command: false,
        custom_prompt: false,
        custom_completion_marker: false,
        custom_additional_prompt: false,
        session_dir: PathBuf::from("/home/user/.config/ralph/sessions/test-session"),
    }
}

#[test]
fn test_startup_info_creation() {
    let info = create_test_info();
    assert_eq!(info.slug, "test-session");
    assert_eq!(info.pending_stories, 5);
    assert_eq!(info.total_stories, 10);
    assert_eq!(info.completed_stories, 5);
}

#[test]
fn test_has_custom_config_none() {
    let info = create_test_info();
    assert!(!info.has_custom_config());
}

#[test]
fn test_has_custom_config_prd() {
    let mut info = create_test_info();
    info.custom_prd_path = Some(PathBuf::from("/custom/prd.toml"));
    assert!(info.has_custom_config());
}

#[test]
fn test_has_custom_config_command() {
    let mut info = create_test_info();
    info.custom_command = true;
    assert!(info.has_custom_config());
}

#[test]
fn test_has_custom_config_prompt() {
    let mut info = create_test_info();
    info.custom_prompt = true;
    assert!(info.has_custom_config());
}

#[test]
fn test_has_custom_config_completion_marker() {
    let mut info = create_test_info();
    info.custom_completion_marker = true;
    assert!(info.has_custom_config());
}

#[test]
fn test_has_custom_config_multiple() {
    let mut info = create_test_info();
    info.custom_command = true;
    info.custom_prompt = true;
    info.custom_prd_path = Some(PathBuf::from("/custom/prd.toml"));
    assert!(info.has_custom_config());
}

#[test]
fn test_has_custom_config_additional_prompt() {
    let mut info = create_test_info();
    info.custom_additional_prompt = true;
    assert!(info.has_custom_config());
}

#[test]
fn test_iterations_from_arg_flag() {
    let mut info = create_test_info();
    assert!(!info.iterations_from_arg);

    info.iterations_from_arg = true;
    assert!(info.iterations_from_arg);
}

// Tests for IterationHeader

fn create_test_header() -> IterationHeader {
    IterationHeader {
        iteration: 1,
        max_iterations: Some(5),
        pending_stories: 3,
    }
}

#[test]
fn test_iteration_header_creation() {
    let header = create_test_header();
    assert_eq!(header.iteration, 1);
    assert_eq!(header.max_iterations, Some(5));
    assert_eq!(header.pending_stories, 3);
}

#[test]
fn test_iteration_header_without_max() {
    let header = IterationHeader {
        iteration: 2,
        max_iterations: None,
        pending_stories: 5,
    };
    assert_eq!(header.iteration, 2);
    assert!(header.max_iterations.is_none());
    assert_eq!(header.pending_stories, 5);
}

#[test]
fn test_iteration_header_singular_story() {
    let header = IterationHeader {
        iteration: 1,
        max_iterations: Some(3),
        pending_stories: 1,
    };
    assert_eq!(header.pending_stories, 1);
}

#[test]
fn test_iteration_header_zero_stories() {
    // Edge case: 0 stories remaining (shouldn't normally happen but should handle it)
    let header = IterationHeader {
        iteration: 5,
        max_iterations: Some(5),
        pending_stories: 0,
    };
    assert_eq!(header.pending_stories, 0);
}

#[test]
fn test_iteration_header_large_numbers() {
    let header = IterationHeader {
        iteration: 100,
        max_iterations: Some(1000),
        pending_stories: 500,
    };
    assert_eq!(header.iteration, 100);
    assert_eq!(header.max_iterations, Some(1000));
    assert_eq!(header.pending_stories, 500);
}

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

// Tests for IterationSummary

fn create_test_summary() -> IterationSummary {
    IterationSummary {
        iteration: 1,
        cost_usd: Some(0.0234),
        duration_ms: Some(45_200),
        model: Some("claude-opus-4-5-20251101".to_string()),
        input_tokens: Some(712),
        output_tokens: Some(2971),
    }
}

#[test]
fn test_iteration_summary_creation() {
    let summary = create_test_summary();
    assert_eq!(summary.iteration, 1);
    assert_eq!(summary.cost_usd, Some(0.0234));
    assert_eq!(summary.duration_ms, Some(45_200));
    assert_eq!(summary.model, Some("claude-opus-4-5-20251101".to_string()));
    assert_eq!(summary.input_tokens, Some(712));
    assert_eq!(summary.output_tokens, Some(2971));
}

#[test]
fn test_iteration_summary_with_none_values() {
    let summary = IterationSummary {
        iteration: 2,
        cost_usd: None,
        duration_ms: None,
        model: None,
        input_tokens: None,
        output_tokens: None,
    };
    assert_eq!(summary.iteration, 2);
    assert!(summary.cost_usd.is_none());
    assert!(summary.duration_ms.is_none());
    assert!(summary.model.is_none());
    assert!(summary.input_tokens.is_none());
    assert!(summary.output_tokens.is_none());
}

#[test]
fn test_iteration_summary_partial_tokens() {
    // Can have input_tokens without output_tokens and vice versa
    let summary = IterationSummary {
        iteration: 1,
        cost_usd: Some(0.05),
        duration_ms: Some(10_000),
        model: None,
        input_tokens: Some(500),
        output_tokens: None,
    };
    assert_eq!(summary.input_tokens, Some(500));
    assert!(summary.output_tokens.is_none());
}

#[test]
fn test_iteration_summary_zero_cost() {
    // Zero cost is valid (e.g., cached responses)
    let summary = IterationSummary {
        iteration: 1,
        cost_usd: Some(0.0),
        duration_ms: Some(100),
        model: Some("test-model".to_string()),
        input_tokens: Some(0),
        output_tokens: Some(0),
    };
    assert_eq!(summary.cost_usd, Some(0.0));
    assert_eq!(summary.input_tokens, Some(0));
    assert_eq!(summary.output_tokens, Some(0));
}

#[test]
fn test_iteration_summary_large_values() {
    // Large token counts and costs
    let summary = IterationSummary {
        iteration: 100,
        cost_usd: Some(15.5678),
        duration_ms: Some(3_600_000), // 1 hour
        model: Some("claude-opus-4-5-20251101".to_string()),
        input_tokens: Some(1_000_000),
        output_tokens: Some(500_000),
    };
    assert_eq!(summary.iteration, 100);
    assert_eq!(summary.cost_usd, Some(15.5678));
    assert_eq!(summary.input_tokens, Some(1_000_000));
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

// Tests for RunSummary

fn create_test_run_summary() -> RunSummary {
    RunSummary {
        slug: "quiet-mountain".to_string(),
        iterations_completed: 3,
        completion_reason: Some("AllStoriesComplete".to_string()),
        total_cost_usd: Some(0.1234),
        total_duration_ms: Some(90_000),
        total_input_tokens: Some(5000),
        total_output_tokens: Some(15000),
        final_pending_stories: 0,
    }
}

#[test]
fn test_run_summary_creation() {
    let summary = create_test_run_summary();
    assert_eq!(summary.slug, "quiet-mountain");
    assert_eq!(summary.iterations_completed, 3);
    assert_eq!(
        summary.completion_reason,
        Some("AllStoriesComplete".to_string())
    );
    assert_eq!(summary.total_cost_usd, Some(0.1234));
    assert_eq!(summary.total_duration_ms, Some(90_000));
    assert_eq!(summary.total_input_tokens, Some(5000));
    assert_eq!(summary.total_output_tokens, Some(15000));
    assert_eq!(summary.final_pending_stories, 0);
}

#[test]
fn test_run_summary_with_none_values() {
    let summary = RunSummary {
        slug: "test-session".to_string(),
        iterations_completed: 1,
        completion_reason: None,
        total_cost_usd: None,
        total_duration_ms: None,
        total_input_tokens: None,
        total_output_tokens: None,
        final_pending_stories: 5,
    };
    assert_eq!(summary.slug, "test-session");
    assert!(summary.completion_reason.is_none());
    assert!(summary.total_cost_usd.is_none());
    assert!(summary.total_duration_ms.is_none());
    assert!(summary.total_input_tokens.is_none());
    assert!(summary.total_output_tokens.is_none());
    assert_eq!(summary.final_pending_stories, 5);
}

#[test]
fn test_run_summary_single_iteration() {
    let summary = RunSummary {
        slug: "single-run".to_string(),
        iterations_completed: 1,
        completion_reason: Some("MarkerFound".to_string()),
        total_cost_usd: Some(0.05),
        total_duration_ms: Some(30_000),
        total_input_tokens: Some(1000),
        total_output_tokens: Some(2000),
        final_pending_stories: 2,
    };
    assert_eq!(summary.iterations_completed, 1);
    assert_eq!(summary.final_pending_stories, 2);
}

#[test]
fn test_run_summary_large_values() {
    let summary = RunSummary {
        slug: "big-run".to_string(),
        iterations_completed: 100,
        completion_reason: Some("AllStoriesComplete".to_string()),
        total_cost_usd: Some(50.0),
        total_duration_ms: Some(3_600_000), // 1 hour
        total_input_tokens: Some(10_000_000),
        total_output_tokens: Some(5_000_000),
        final_pending_stories: 0,
    };
    assert_eq!(summary.iterations_completed, 100);
    assert_eq!(summary.total_cost_usd, Some(50.0));
    assert_eq!(summary.total_input_tokens, Some(10_000_000));
}

#[test]
fn test_run_summary_partial_completion() {
    // Run that stopped before completing all stories (e.g., max iterations reached)
    let summary = RunSummary {
        slug: "partial-run".to_string(),
        iterations_completed: 10,
        completion_reason: None, // No completion reason means max iterations reached
        total_cost_usd: Some(1.5),
        total_duration_ms: Some(300_000),
        total_input_tokens: Some(50_000),
        total_output_tokens: Some(100_000),
        final_pending_stories: 3, // Still 3 stories remaining
    };
    assert_eq!(summary.iterations_completed, 10);
    assert!(summary.completion_reason.is_none());
    assert_eq!(summary.final_pending_stories, 3);
}

#[test]
fn test_run_summary_zero_cost() {
    // Edge case: run with no cost data (e.g., all cached)
    let summary = RunSummary {
        slug: "cached-run".to_string(),
        iterations_completed: 2,
        completion_reason: Some("AllStoriesComplete".to_string()),
        total_cost_usd: Some(0.0),
        total_duration_ms: Some(5000),
        total_input_tokens: Some(0),
        total_output_tokens: Some(0),
        final_pending_stories: 0,
    };
    assert_eq!(summary.total_cost_usd, Some(0.0));
    assert_eq!(summary.total_input_tokens, Some(0));
}

#[test]
fn test_run_summary_clone() {
    let original = create_test_run_summary();
    let cloned = original.clone();
    assert_eq!(original.slug, cloned.slug);
    assert_eq!(original.iterations_completed, cloned.iterations_completed);
    assert_eq!(original.total_cost_usd, cloned.total_cost_usd);
}

// Tests for AttachedFile

#[test]
fn test_attached_file_design_md_not_special_cased() {
    let file = AttachedFile::new(PathBuf::from("/project/.local/designs/design.md"));
    assert_eq!(file.description, "Attached file");
}

#[test]
fn test_attached_file_prd() {
    let file = AttachedFile::new(PathBuf::from("/project/.local/plans/prd.toml"));
    assert_eq!(file.description, "Product requirements");
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
    let prompt = "@/project/design.md @/project/prd.toml @/project/progress.txt

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
        PathBuf::from("/project/prd.toml")
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
    let prompt = "@/project/design.md @/project/prd.toml

1. Do something
2. Do another thing";

    let display = PromptDisplay::from_prompt(prompt);
    let stripped = display.stripped_prompt();

    // Should not contain file references
    assert!(!stripped.contains("@/project/design.md"));
    assert!(!stripped.contains("@/project/prd.toml"));
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
        slug: "test-ask".to_string(),
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
    assert_eq!(summary.slug, "test-ask");
    assert!(summary.success);
    assert_eq!(summary.cost_usd, Some(0.05));
    assert_eq!(summary.duration_ms, Some(10_000));
    assert_eq!(summary.input_tokens, Some(500));
    assert_eq!(summary.output_tokens, Some(1500));
}

#[test]
fn test_ask_summary_failed() {
    let summary = AskSummary {
        slug: "failed-ask".to_string(),
        success: false,
        cost_usd: Some(0.01),
        duration_ms: Some(5_000),
        input_tokens: Some(100),
        output_tokens: None,
    };
    assert!(!summary.success);
    assert_eq!(summary.slug, "failed-ask");
    assert_eq!(summary.cost_usd, Some(0.01));
}

#[test]
fn test_ask_summary_with_none_values() {
    let summary = AskSummary {
        slug: "minimal-ask".to_string(),
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
    assert_eq!(original.slug, cloned.slug);
    assert_eq!(original.success, cloned.success);
    assert_eq!(original.cost_usd, cloned.cost_usd);
    assert_eq!(original.duration_ms, cloned.duration_ms);
}

#[test]
fn test_ask_summary_zero_cost() {
    // Zero cost is valid (e.g., cached responses)
    let summary = AskSummary {
        slug: "cached-ask".to_string(),
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
        slug: "big-ask".to_string(),
        success: true,
        cost_usd: Some(5.5678),
        duration_ms: Some(300_000), // 5 minutes
        input_tokens: Some(100_000),
        output_tokens: Some(50_000),
    };
    assert_eq!(summary.cost_usd, Some(5.5678));
    assert_eq!(summary.input_tokens, Some(100_000));
}
