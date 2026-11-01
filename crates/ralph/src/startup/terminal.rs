//! Terminal-formatted output with ANSI escape codes.

use super::formatters::{format_duration, format_token_count};
use super::types::{AskSummary, PromptDisplay};
use crate::ansi::{fg, BOLD, DIM, RESET};
use crate::markdown::MarkdownRenderer;
use crate::ui_theme::UiColors;

pub(super) fn prompt_rule(ui: &UiColors, width: usize) -> String {
    format!("{}{}{RESET}", fg(Some(ui.ask_header)), "─".repeat(width))
}

pub(super) fn prompt_header(ui: &UiColors, width: usize) -> String {
    format!(
        "{BOLD}{}▶ Prompt{RESET}\n{}",
        fg(Some(ui.ask_header)),
        prompt_rule(ui, width)
    )
}

pub(super) fn attached_file_line(ui: &UiColors, path: &str, description: &str) -> String {
    format!(
        "  {}{path}{RESET}  {DIM}→{RESET}  {description}",
        fg(Some(ui.ask_path))
    )
}

pub(super) fn ask_status_line(ui: &UiColors, success: bool) -> String {
    let (icon, color, text) = if success {
        ("✓", fg(Some(ui.ask_ok)), "Complete")
    } else {
        ("✗", fg(Some(ui.ask_error)), "Failed")
    };
    format!("{color}{icon}{RESET} {color}Ask {text}{RESET}")
}

/// Display prompt with terminal formatting.
pub(super) fn display_prompt_terminal(prompt: &PromptDisplay, ui: &UiColors) {
    let term_width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80);
    let width = super::formatters::separator_width(&prompt.stripped_prompt(), term_width);

    // Header with visual indicator
    println!();
    println!("{}", prompt_header(ui, width));

    // Display attached files as a table (if any)
    if !prompt.attached_files.is_empty() {
        println!();
        println!("{DIM}Attached files:{RESET}");
        for file in &prompt.attached_files {
            // Format: "  path  →  Description"
            println!(
                "{}",
                attached_file_line(ui, &file.path.display().to_string(), file.description)
            );
        }
    }

    println!();

    // Render the prompt with markdown formatting (strip @/path references from display)
    let renderer = MarkdownRenderer::new();
    let rendered = renderer.render(&prompt.stripped_prompt());
    println!("{}", rendered);

    // Closing separator
    println!("{}", prompt_rule(ui, width));
}

/// Display ask summary with terminal formatting.
pub(super) fn display_ask_summary_terminal(summary: &AskSummary, ui: &UiColors) {
    println!();

    println!("{}", ask_status_line(ui, summary.success));
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
