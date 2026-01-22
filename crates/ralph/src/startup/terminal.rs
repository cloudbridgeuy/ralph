//! Terminal-formatted output with ANSI escape codes.

use super::formatters::{format_duration, format_token_count};
use super::types::{
    AskSummary, IterationHeader, IterationSummary, PromptDisplay, RunSummary, StartupInfo, VERSION,
};
use crate::markdown::MarkdownRenderer;

/// Display startup info with terminal formatting.
pub(super) fn display_startup_terminal(info: &StartupInfo) {
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

/// Display iteration header with terminal formatting.
pub(super) fn display_iteration_header_terminal(header: &IterationHeader) {
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

/// Display iteration summary with terminal formatting.
pub(super) fn display_iteration_summary_terminal(summary: &IterationSummary) {
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

/// Display final run summary with terminal formatting.
pub(super) fn display_run_summary_terminal(summary: &RunSummary) {
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

/// Display prompt with terminal formatting.
pub(super) fn display_prompt_terminal(prompt: &PromptDisplay) {
    // Header with visual indicator
    println!();
    println!("\x1b[1m\x1b[35m▶ Prompt\x1b[0m");
    println!("\x1b[35m{}\x1b[0m", "─".repeat(60));

    // Display attached files as a table (if any)
    if !prompt.attached_files.is_empty() {
        println!();
        println!("\x1b[2mAttached files:\x1b[0m");
        for file in &prompt.attached_files {
            // Format: "  path  →  Description"
            println!(
                "  \x1b[36m{}\x1b[0m  \x1b[2m→\x1b[0m  {}",
                file.path.display(),
                file.description
            );
        }
    }

    println!();

    // Render the prompt with markdown formatting (strip @/path references from display)
    let renderer = MarkdownRenderer::new();
    let rendered = renderer.render(&prompt.stripped_prompt());
    println!("{}", rendered);

    // Closing separator
    println!("\x1b[35m{}\x1b[0m", "─".repeat(60));
}

/// Display ask summary with terminal formatting.
pub(super) fn display_ask_summary_terminal(summary: &AskSummary) {
    println!();

    // Status indicator based on success
    let (status_icon, status_color) = if summary.success {
        ("\x1b[32m✓\x1b[0m", "\x1b[32m") // Green checkmark
    } else {
        ("\x1b[31m✗\x1b[0m", "\x1b[31m") // Red X
    };

    // Header with status
    let status_text = if summary.success {
        "Complete"
    } else {
        "Failed"
    };
    println!("{} {}Ask {}\x1b[0m", status_icon, status_color, status_text);
    println!();

    // Session slug (prominently displayed for replay)
    println!("\x1b[1mSession:\x1b[0m \x1b[33m{}\x1b[0m", summary.slug);

    // Metrics section
    println!();
    println!("\x1b[2m─── Metrics ───\x1b[0m");

    // Cost
    let cost_str = summary
        .cost_usd
        .map(|c| format!("${:.4}", c))
        .unwrap_or_else(|| "N/A".to_string());
    println!("\x1b[2mCost:\x1b[0m {}", cost_str);

    // Duration
    let duration_str = summary
        .duration_ms
        .map(format_duration)
        .unwrap_or_else(|| "N/A".to_string());
    println!("\x1b[2mDuration:\x1b[0m {}", duration_str);

    // Tokens
    let has_tokens = summary.input_tokens.is_some() || summary.output_tokens.is_some();
    if has_tokens {
        let input_str = summary
            .input_tokens
            .map(format_token_count)
            .unwrap_or_else(|| "N/A".to_string());
        let output_str = summary
            .output_tokens
            .map(format_token_count)
            .unwrap_or_else(|| "N/A".to_string());
        println!(
            "\x1b[2mTokens:\x1b[0m {} input | {} output",
            input_str, output_str
        );
    }

    println!();
    println!("\x1b[2mReplay with: ralph replay {}\x1b[0m", summary.slug);
    println!();
}
