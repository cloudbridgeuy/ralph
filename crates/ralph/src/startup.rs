//! Startup information display for the run command.
//!
//! This module provides functions to display startup information when
//! `ralph run` begins, giving users immediate feedback about the session
//! being created and the work to be done.

use std::io::IsTerminal;
use std::path::PathBuf;

/// Version of the ralph binary (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Information to display at startup.
#[derive(Debug)]
pub struct StartupInfo {
    /// The session slug (generated or user-provided).
    pub slug: String,
    /// Total number of stories in the PRD.
    pub total_stories: usize,
    /// Number of pending stories.
    pub pending_stories: usize,
    /// Number of completed stories.
    pub completed_stories: usize,
    /// Maximum iterations to run.
    pub max_iterations: usize,
    /// Whether max_iterations was explicitly provided by user.
    pub iterations_from_arg: bool,
    /// Path to PRD file (only shown if custom).
    pub custom_prd_path: Option<PathBuf>,
    /// Path to design file (only shown if custom).
    pub custom_design_path: Option<PathBuf>,
    /// Path to progress file (only shown if custom).
    pub custom_progress_path: Option<PathBuf>,
    /// Whether a custom command template is used.
    pub custom_command: bool,
    /// Whether a custom prompt is used.
    pub custom_prompt: bool,
    /// Whether a custom completion marker is used.
    pub custom_completion_marker: bool,
    /// Whether an additional prompt was provided.
    pub custom_additional_prompt: bool,
    /// Session directory path.
    pub session_dir: PathBuf,
}

impl StartupInfo {
    /// Check if any custom configuration is being used.
    fn has_custom_config(&self) -> bool {
        self.custom_prd_path.is_some()
            || self.custom_design_path.is_some()
            || self.custom_progress_path.is_some()
            || self.custom_command
            || self.custom_prompt
            || self.custom_completion_marker
            || self.custom_additional_prompt
    }
}

/// Display startup information to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses box drawing characters and colors
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_startup_info(info: &StartupInfo) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        display_startup_terminal(info);
    } else {
        display_startup_plain(info);
    }
}

/// Display startup info with terminal formatting.
fn display_startup_terminal(info: &StartupInfo) {
    // Header with version
    println!();
    println!("\x1b[1m\x1b[36m━━━ ralph v{} ━━━\x1b[0m", VERSION);
    println!();

    // Session info
    println!("\x1b[1mSession:\x1b[0m \x1b[33m{}\x1b[0m", info.slug);

    // PRD status
    println!(
        "\x1b[1mPRD:\x1b[0m {} pending / {} total ({} completed)",
        info.pending_stories, info.total_stories, info.completed_stories
    );

    // Iterations
    let iterations_note = if info.iterations_from_arg {
        "(from argument)"
    } else {
        "(auto: pending count)"
    };
    println!(
        "\x1b[1mIterations:\x1b[0m up to {} {}",
        info.max_iterations, iterations_note
    );

    // Custom config (only if any overrides present)
    if info.has_custom_config() {
        println!();
        println!("\x1b[2mCustom configuration:\x1b[0m");
        if let Some(ref path) = info.custom_prd_path {
            println!("  \x1b[2m--prd {}\x1b[0m", path.display());
        }
        if let Some(ref path) = info.custom_design_path {
            println!("  \x1b[2m--design {}\x1b[0m", path.display());
        }
        if let Some(ref path) = info.custom_progress_path {
            println!("  \x1b[2m--progress {}\x1b[0m", path.display());
        }
        if info.custom_command {
            println!("  \x1b[2m--command (custom)\x1b[0m");
        }
        if info.custom_prompt {
            println!("  \x1b[2m--prompt (custom)\x1b[0m");
        }
        if info.custom_completion_marker {
            println!("  \x1b[2m--completion-marker (custom)\x1b[0m");
        }
        if info.custom_additional_prompt {
            println!("  \x1b[2m--additional-prompt (custom)\x1b[0m");
        }
    }

    // Session directory
    println!();
    println!("\x1b[2mLogs: {}\x1b[0m", info.session_dir.display());

    // Separator before first iteration
    println!();
    println!("\x1b[36m{}\x1b[0m", "─".repeat(60));
    println!();
}

/// Display startup info without terminal formatting.
fn display_startup_plain(info: &StartupInfo) {
    // Header with version
    println!();
    println!("=== ralph v{} ===", VERSION);
    println!();

    // Session info
    println!("Session: {}", info.slug);

    // PRD status
    println!(
        "PRD: {} pending / {} total ({} completed)",
        info.pending_stories, info.total_stories, info.completed_stories
    );

    // Iterations
    let iterations_note = if info.iterations_from_arg {
        "(from argument)"
    } else {
        "(auto: pending count)"
    };
    println!(
        "Iterations: up to {} {}",
        info.max_iterations, iterations_note
    );

    // Custom config (only if any overrides present)
    if info.has_custom_config() {
        println!();
        println!("Custom configuration:");
        if let Some(ref path) = info.custom_prd_path {
            println!("  --prd {}", path.display());
        }
        if let Some(ref path) = info.custom_design_path {
            println!("  --design {}", path.display());
        }
        if let Some(ref path) = info.custom_progress_path {
            println!("  --progress {}", path.display());
        }
        if info.custom_command {
            println!("  --command (custom)");
        }
        if info.custom_prompt {
            println!("  --prompt (custom)");
        }
        if info.custom_completion_marker {
            println!("  --completion-marker (custom)");
        }
        if info.custom_additional_prompt {
            println!("  --additional-prompt (custom)");
        }
    }

    // Session directory
    println!();
    println!("Logs: {}", info.session_dir.display());

    // Separator before first iteration
    println!();
    println!("{}", "-".repeat(60));
    println!();
}

/// Information for iteration header display.
#[derive(Debug)]
pub struct IterationHeader {
    /// Current iteration number (1-indexed).
    pub iteration: usize,
    /// Maximum iterations, if known.
    pub max_iterations: Option<usize>,
    /// Number of pending stories at the start of this iteration.
    pub pending_stories: usize,
}

/// Display iteration header to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses box drawing characters and colors
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_iteration_header(header: &IterationHeader) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        display_iteration_header_terminal(header);
    } else {
        display_iteration_header_plain(header);
    }
}

/// Display iteration header with terminal formatting.
fn display_iteration_header_terminal(header: &IterationHeader) {
    // Iteration indicator
    let iteration_text = match header.max_iterations {
        Some(max) => format!("Iteration {}/{}", header.iteration, max),
        None => format!("Iteration {}", header.iteration),
    };

    // Story count
    let stories_text = if header.pending_stories == 1 {
        "1 story remaining".to_string()
    } else {
        format!("{} stories remaining", header.pending_stories)
    };

    // Print header with visual separator
    println!();
    println!(
        "\x1b[1m\x1b[34m━━━ {} • {} ━━━\x1b[0m",
        iteration_text, stories_text
    );
    println!();
}

/// Display iteration header without terminal formatting.
fn display_iteration_header_plain(header: &IterationHeader) {
    // Iteration indicator
    let iteration_text = match header.max_iterations {
        Some(max) => format!("Iteration {}/{}", header.iteration, max),
        None => format!("Iteration {}", header.iteration),
    };

    // Story count
    let stories_text = if header.pending_stories == 1 {
        "1 story remaining".to_string()
    } else {
        format!("{} stories remaining", header.pending_stories)
    };

    // Print header with visual separator
    println!();
    println!("--- {} | {} ---", iteration_text, stories_text);
    println!();
}

/// Information for iteration summary display.
#[derive(Debug)]
pub struct IterationSummary {
    /// Current iteration number (1-indexed).
    pub iteration: usize,
    /// Cost in USD for this iteration (from result event).
    pub cost_usd: Option<f64>,
    /// Duration in milliseconds (from result event).
    pub duration_ms: Option<u64>,
    /// Model name used for this iteration.
    pub model: Option<String>,
    /// Input tokens used.
    pub input_tokens: Option<u64>,
    /// Output tokens generated.
    pub output_tokens: Option<u64>,
}

/// Display iteration summary to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses dimmed colors for a subtle summary appearance
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_iteration_summary(summary: &IterationSummary) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        display_iteration_summary_terminal(summary);
    } else {
        display_iteration_summary_plain(summary);
    }
}

/// Display iteration summary with terminal formatting.
fn display_iteration_summary_terminal(summary: &IterationSummary) {
    println!();
    println!(
        "\x1b[2m─── Iteration {} Summary ───\x1b[0m",
        summary.iteration
    );

    // Cost and duration on one line
    let cost_str = summary
        .cost_usd
        .map(|c| format!("${:.4}", c))
        .unwrap_or_else(|| "N/A".to_string());
    let duration_str = summary
        .duration_ms
        .map(format_duration)
        .unwrap_or_else(|| "N/A".to_string());

    println!(
        "\x1b[2mCost: {} • Duration: {}\x1b[0m",
        cost_str, duration_str
    );

    // Model
    if let Some(ref model) = summary.model {
        println!("\x1b[2mModel: {}\x1b[0m", model);
    }

    // Tokens
    let has_tokens = summary.input_tokens.is_some() || summary.output_tokens.is_some();
    if has_tokens {
        let input_str = summary
            .input_tokens
            .map(|t| t.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        let output_str = summary
            .output_tokens
            .map(|t| t.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        println!(
            "\x1b[2mTokens: {} input | {} output\x1b[0m",
            input_str, output_str
        );
    }

    println!("\x1b[2m{}\x1b[0m", "─".repeat(30));
}

/// Display iteration summary without terminal formatting.
fn display_iteration_summary_plain(summary: &IterationSummary) {
    println!();
    println!("--- Iteration {} Summary ---", summary.iteration);

    // Cost and duration on one line
    let cost_str = summary
        .cost_usd
        .map(|c| format!("${:.4}", c))
        .unwrap_or_else(|| "N/A".to_string());
    let duration_str = summary
        .duration_ms
        .map(format_duration)
        .unwrap_or_else(|| "N/A".to_string());

    println!("Cost: {} | Duration: {}", cost_str, duration_str);

    // Model
    if let Some(ref model) = summary.model {
        println!("Model: {}", model);
    }

    // Tokens
    let has_tokens = summary.input_tokens.is_some() || summary.output_tokens.is_some();
    if has_tokens {
        let input_str = summary
            .input_tokens
            .map(|t| t.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        let output_str = summary
            .output_tokens
            .map(|t| t.to_string())
            .unwrap_or_else(|| "N/A".to_string());
        println!("Tokens: {} input | {} output", input_str, output_str);
    }

    println!("{}", "-".repeat(30));
}

/// Information for final run summary display.
#[derive(Debug, Clone)]
pub struct RunSummary {
    /// The session slug.
    pub slug: String,
    /// Total number of iterations completed.
    pub iterations_completed: usize,
    /// Reason for completion (if any).
    pub completion_reason: Option<String>,
    /// Total cost across all iterations (USD).
    pub total_cost_usd: Option<f64>,
    /// Total duration across all iterations (wall clock time from start).
    pub total_duration_ms: Option<u64>,
    /// Total input tokens across all iterations.
    pub total_input_tokens: Option<u64>,
    /// Total output tokens across all iterations.
    pub total_output_tokens: Option<u64>,
    /// Final pending story count.
    pub final_pending_stories: usize,
}

/// Display final run summary to stdout.
///
/// The output format adapts based on whether stdout is a terminal:
/// - Terminal: Uses box drawing characters and colors with clear boundaries
/// - Piped: Uses plain ASCII with no ANSI codes
pub fn display_run_summary(summary: &RunSummary) {
    let is_terminal = std::io::stdout().is_terminal();

    if is_terminal {
        display_run_summary_terminal(summary);
    } else {
        display_run_summary_plain(summary);
    }
}

/// Display final run summary with terminal formatting.
fn display_run_summary_terminal(summary: &RunSummary) {
    println!();
    println!("\x1b[1m\x1b[36m╔══════════════════════════════════════════════════════════╗\x1b[0m");
    println!("\x1b[1m\x1b[36m║                    Run Complete                          ║\x1b[0m");
    println!("\x1b[1m\x1b[36m╚══════════════════════════════════════════════════════════╝\x1b[0m");
    println!();

    // Session slug (prominently displayed for replay)
    println!("\x1b[1mSession:\x1b[0m \x1b[33m{}\x1b[0m", summary.slug);

    // Iterations and completion reason
    let iterations_text = if summary.iterations_completed == 1 {
        "1 iteration".to_string()
    } else {
        format!("{} iterations", summary.iterations_completed)
    };
    println!("\x1b[1mIterations:\x1b[0m {}", iterations_text);

    // Completion reason
    if let Some(ref reason) = summary.completion_reason {
        let reason_display = match reason.as_str() {
            "AllStoriesComplete" => "\x1b[32mAll stories complete\x1b[0m",
            "MarkerFound" => "\x1b[32mCompletion marker found\x1b[0m",
            _ => reason.as_str(),
        };
        println!("\x1b[1mResult:\x1b[0m {}", reason_display);
    }

    // Final story count
    let stories_text = if summary.final_pending_stories == 0 {
        "\x1b[32m0 stories remaining\x1b[0m".to_string()
    } else if summary.final_pending_stories == 1 {
        "\x1b[33m1 story remaining\x1b[0m".to_string()
    } else {
        format!(
            "\x1b[33m{} stories remaining\x1b[0m",
            summary.final_pending_stories
        )
    };
    println!("\x1b[1mStories:\x1b[0m {}", stories_text);

    println!();

    // Aggregated totals section
    println!("\x1b[2m─── Totals ───\x1b[0m");

    // Total cost
    let cost_str = summary
        .total_cost_usd
        .map(|c| format!("${:.4}", c))
        .unwrap_or_else(|| "N/A".to_string());
    println!("\x1b[2mTotal cost:\x1b[0m {}", cost_str);

    // Total duration
    let duration_str = summary
        .total_duration_ms
        .map(format_duration)
        .unwrap_or_else(|| "N/A".to_string());
    println!("\x1b[2mTotal duration:\x1b[0m {}", duration_str);

    // Total tokens
    let has_tokens = summary.total_input_tokens.is_some() || summary.total_output_tokens.is_some();
    if has_tokens {
        let input_str = summary
            .total_input_tokens
            .map(format_token_count)
            .unwrap_or_else(|| "N/A".to_string());
        let output_str = summary
            .total_output_tokens
            .map(format_token_count)
            .unwrap_or_else(|| "N/A".to_string());
        println!(
            "\x1b[2mTotal tokens:\x1b[0m {} input | {} output",
            input_str, output_str
        );
    }

    println!();
    println!("\x1b[2mReplay with: ralph replay {}\x1b[0m", summary.slug);
    println!();
}

/// Display final run summary without terminal formatting.
fn display_run_summary_plain(summary: &RunSummary) {
    println!();
    println!("============================================================");
    println!("                     Run Complete                           ");
    println!("============================================================");
    println!();

    // Session slug
    println!("Session: {}", summary.slug);

    // Iterations and completion reason
    let iterations_text = if summary.iterations_completed == 1 {
        "1 iteration".to_string()
    } else {
        format!("{} iterations", summary.iterations_completed)
    };
    println!("Iterations: {}", iterations_text);

    // Completion reason
    if let Some(ref reason) = summary.completion_reason {
        let reason_display = match reason.as_str() {
            "AllStoriesComplete" => "All stories complete",
            "MarkerFound" => "Completion marker found",
            _ => reason.as_str(),
        };
        println!("Result: {}", reason_display);
    }

    // Final story count
    let stories_text = if summary.final_pending_stories == 0 {
        "0 stories remaining".to_string()
    } else if summary.final_pending_stories == 1 {
        "1 story remaining".to_string()
    } else {
        format!("{} stories remaining", summary.final_pending_stories)
    };
    println!("Stories: {}", stories_text);

    println!();

    // Aggregated totals section
    println!("--- Totals ---");

    // Total cost
    let cost_str = summary
        .total_cost_usd
        .map(|c| format!("${:.4}", c))
        .unwrap_or_else(|| "N/A".to_string());
    println!("Total cost: {}", cost_str);

    // Total duration
    let duration_str = summary
        .total_duration_ms
        .map(format_duration)
        .unwrap_or_else(|| "N/A".to_string());
    println!("Total duration: {}", duration_str);

    // Total tokens
    let has_tokens = summary.total_input_tokens.is_some() || summary.total_output_tokens.is_some();
    if has_tokens {
        let input_str = summary
            .total_input_tokens
            .map(format_token_count)
            .unwrap_or_else(|| "N/A".to_string());
        let output_str = summary
            .total_output_tokens
            .map(format_token_count)
            .unwrap_or_else(|| "N/A".to_string());
        println!("Total tokens: {} input | {} output", input_str, output_str);
    }

    println!();
    println!("Replay with: ralph replay {}", summary.slug);
    println!();
}

/// Format token count with thousands separators for readability.
fn format_token_count(tokens: u64) -> String {
    if tokens < 1000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}K", tokens as f64 / 1000.0)
    } else {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    }
}

/// Format duration from milliseconds to human-readable string.
///
/// # Formatting rules
/// - 0-999ms → "Xms"
/// - 1000-59999ms → "X.Xs" (e.g., "45.2s")
/// - 60000+ ms → "Xm Ys" (e.g., "1m 23s")
pub fn format_duration(duration_ms: u64) -> String {
    if duration_ms < 1000 {
        format!("{}ms", duration_ms)
    } else if duration_ms < 60_000 {
        let seconds = duration_ms as f64 / 1000.0;
        format!("{:.1}s", seconds)
    } else {
        let total_seconds = duration_ms / 1000;
        let minutes = total_seconds / 60;
        let seconds = total_seconds % 60;
        format!("{}m {}s", minutes, seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_info() -> StartupInfo {
        StartupInfo {
            slug: "test-session".to_string(),
            total_stories: 10,
            pending_stories: 5,
            completed_stories: 5,
            max_iterations: 5,
            iterations_from_arg: false,
            custom_prd_path: None,
            custom_design_path: None,
            custom_progress_path: None,
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
    fn test_has_custom_config_design() {
        let mut info = create_test_info();
        info.custom_design_path = Some(PathBuf::from("/custom/design.md"));
        assert!(info.has_custom_config());
    }

    #[test]
    fn test_has_custom_config_progress() {
        let mut info = create_test_info();
        info.custom_progress_path = Some(PathBuf::from("/custom/progress.txt"));
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
}
