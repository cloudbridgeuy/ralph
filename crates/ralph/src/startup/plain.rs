//! Plain text output without ANSI formatting.

use super::formatters::{format_duration, format_token_count};
use super::types::{IterationHeader, IterationSummary, RunSummary, StartupInfo, VERSION};

/// Display startup info without terminal formatting.
pub(super) fn display_startup_plain(info: &StartupInfo) {
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

/// Display iteration header without terminal formatting.
pub(super) fn display_iteration_header_plain(header: &IterationHeader) {
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

/// Display iteration summary without terminal formatting.
pub(super) fn display_iteration_summary_plain(summary: &IterationSummary) {
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

/// Display final run summary without terminal formatting.
pub(super) fn display_run_summary_plain(summary: &RunSummary) {
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
