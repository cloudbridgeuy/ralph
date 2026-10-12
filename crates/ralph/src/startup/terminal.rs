//! Terminal-formatted output with ANSI escape codes.

use super::formatters::{format_duration, format_token_count};
use super::types::{AskSummary, PromptDisplay};
use crate::markdown::MarkdownRenderer;

/// Display prompt with terminal formatting.
pub(super) fn display_prompt_terminal(prompt: &PromptDisplay) {
    let term_width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80);
    let width = super::formatters::separator_width(&prompt.stripped_prompt(), term_width);

    // Header with visual indicator
    println!();
    println!("\x1b[1m\x1b[35m▶ Prompt\x1b[0m");
    println!("\x1b[35m{}\x1b[0m", "─".repeat(width));

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
    println!("\x1b[35m{}\x1b[0m", "─".repeat(width));
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
}
