//! Plain text output without ANSI formatting.

use super::formatters::{format_duration, format_token_count};
use super::types::{AskSummary, PromptDisplay};

/// Display prompt without terminal formatting.
pub(super) fn display_prompt_plain(prompt: &PromptDisplay) {
    let term_width: u16 = 80;
    let width = super::formatters::separator_width(&prompt.stripped_prompt(), term_width);

    // Header
    println!();
    println!("> Prompt");
    println!("{}", "-".repeat(width));

    // Display attached files (if any)
    if !prompt.attached_files.is_empty() {
        println!();
        println!("Attached files:");
        for file in &prompt.attached_files {
            println!("  {} -> {}", file.path.display(), file.description);
        }
    }

    println!();

    // Print the prompt with file references stripped (no markdown rendering)
    println!("{}", prompt.stripped_prompt());

    // Closing separator
    println!("{}", "-".repeat(width));
}

/// Display ask summary without terminal formatting.
pub(super) fn display_ask_summary_plain(summary: &AskSummary) {
    println!();

    // Status
    let status_text = if summary.success {
        "Ask Complete"
    } else {
        "Ask Failed"
    };
    println!("{}", status_text);
    println!();
    println!("--- Metrics ---");

    // Cost
    let cost_str = summary
        .cost_usd
        .map(|c| format!("${:.4}", c))
        .unwrap_or_else(|| "N/A".to_string());
    println!("Cost: {}", cost_str);

    // Duration
    let duration_str = summary
        .duration_ms
        .map(format_duration)
        .unwrap_or_else(|| "N/A".to_string());
    println!("Duration: {}", duration_str);

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
        println!("Tokens: {} input | {} output", input_str, output_str);
    }

    println!();
}
