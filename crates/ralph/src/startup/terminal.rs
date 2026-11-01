//! Terminal-formatted output with ANSI escape codes.

use super::formatters::{format_duration, format_token_count};
use super::types::{AskSummary, PromptDisplay};
use crate::ansi::{BOLD, CYAN, DIM, GREEN, MAGENTA, RED, RESET};
use crate::markdown::MarkdownRenderer;

/// Display prompt with terminal formatting.
pub(super) fn display_prompt_terminal(prompt: &PromptDisplay) {
    let term_width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80);
    let width = super::formatters::separator_width(&prompt.stripped_prompt(), term_width);

    // Header with visual indicator
    println!();
    println!("{BOLD}{MAGENTA}▶ Prompt{RESET}");
    println!("{MAGENTA}{}{RESET}", "─".repeat(width));

    // Display attached files as a table (if any)
    if !prompt.attached_files.is_empty() {
        println!();
        println!("{DIM}Attached files:{RESET}");
        for file in &prompt.attached_files {
            // Format: "  path  →  Description"
            println!(
                "  {CYAN}{}{RESET}  {DIM}→{RESET}  {}",
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
    println!("{MAGENTA}{}{RESET}", "─".repeat(width));
}

/// Display ask summary with terminal formatting.
pub(super) fn display_ask_summary_terminal(summary: &AskSummary) {
    println!();

    // Status indicator based on success
    let (status_icon, status_color) = if summary.success {
        (format!("{GREEN}✓{RESET}"), GREEN) // Green checkmark
    } else {
        (format!("{RED}✗{RESET}"), RED) // Red X
    };

    // Header with status
    let status_text = if summary.success {
        "Complete"
    } else {
        "Failed"
    };
    println!("{} {}Ask {}{RESET}", status_icon, status_color, status_text);
    println!();
    println!("{DIM}─── Metrics ───{RESET}");

    // Cost
    let cost_str = summary
        .cost_usd
        .map(|c| format!("${:.4}", c))
        .unwrap_or_else(|| "N/A".to_string());
    println!("{DIM}Cost:{RESET} {}", cost_str);

    // Duration
    let duration_str = summary
        .duration_ms
        .map(format_duration)
        .unwrap_or_else(|| "N/A".to_string());
    println!("{DIM}Duration:{RESET} {}", duration_str);

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
            "{DIM}Tokens:{RESET} {} input | {} output",
            input_str, output_str
        );
    }

    println!();
}
