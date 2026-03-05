//! Display functions for orchestration routing status and summaries.
//!
//! Follows Functional Core - Imperative Shell: `format_*` functions are pure
//! (return `String`), `print_*` functions handle I/O.

use super::Budget;
use crate::ansi::{CYAN, DIM, GREEN, RESET};
use crate::markdown::MarkdownRenderer;
use crate::startup::formatters::separator_width;
use ralph_core::directive::DirectiveVerb;

/// Format a routing status line showing the directive being executed.
///
/// Pure function — returns a styled string with ANSI codes.
///
/// # Format
///
/// ```text
/// ▶ pm → ask → architect                                    [9/10]
/// ```
fn format_routing_status(
    originator: &str,
    verb: &DirectiveVerb,
    target: &str,
    budget: &Budget,
) -> String {
    let verb_str = match verb {
        DirectiveVerb::Ask => "ask",
        DirectiveVerb::Handover => "handover",
    };

    let header =
        format!("{CYAN}▶{RESET} {CYAN}{originator}{RESET} → {verb_str} → {CYAN}{target}{RESET}");
    let budget_display = format!("{DIM}[{}/{}]{RESET}", budget.remaining(), budget.limit());

    format!("{header}  {budget_display}")
}

/// Print a routing status line to stdout.
///
/// Imperative shell wrapper around [`format_routing_status`].
pub fn print_routing_status(originator: &str, verb: &DirectiveVerb, target: &str, budget: &Budget) {
    println!(
        "{}",
        format_routing_status(originator, verb, target, budget)
    );
}

/// Format a directive payload as a markdown banner with separator lines.
///
/// Pure function — returns the full banner string with ANSI codes.
fn format_directive_banner(payload: &str, terminal_width: u16) -> String {
    let width = separator_width(payload, terminal_width);
    let separator = format!("{DIM}{}{RESET}", "─".repeat(width));

    let renderer = MarkdownRenderer::new();
    let rendered = renderer.render(payload);

    format!("{separator}\n\n{rendered}\n\n{separator}")
}

/// Print a directive payload as a markdown banner to stdout.
///
/// Imperative shell wrapper around [`format_directive_banner`].
pub fn print_directive_banner(payload: &str) {
    let term_width = crossterm::terminal::size().map(|(w, _)| w).unwrap_or(80);
    println!("{}", format_directive_banner(payload, term_width));
}

/// Format an orchestration completion summary.
///
/// Pure function — returns a styled string with ANSI codes.
///
/// # Format
///
/// ```text
/// ✓ Orchestration complete                                   [7/10]
/// ```
fn format_orchestration_summary(budget: &Budget) -> String {
    let remaining = budget.remaining();
    let limit = budget.limit();
    format!("{GREEN}✓{RESET} Orchestration complete  {DIM}[{remaining}/{limit}]{RESET}")
}

/// Print an orchestration completion summary to stdout.
///
/// Imperative shell wrapper around [`format_orchestration_summary`].
pub fn print_orchestration_summary(budget: &Budget) {
    println!("{}", format_orchestration_summary(budget));
}

/// Format a persona banner showing which persona is about to stream.
///
/// Pure function — returns a styled string with ANSI codes.
///
/// # Format
///
/// ```text
/// ━━━ architect ━━━
/// ```
fn format_persona_banner(name: &str) -> String {
    format!("\n{CYAN}━━━{RESET} {CYAN}{name}{RESET} {CYAN}━━━{RESET}\n")
}

/// Print a persona banner to stdout.
///
/// Imperative shell wrapper around [`format_persona_banner`].
pub fn print_persona_banner(name: &str) {
    println!("{}", format_persona_banner(name));
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // format_routing_status tests
    // =========================================================================

    #[test]
    fn format_routing_status_ask() {
        let budget = Budget::new(10);
        let result = format_routing_status("pm", &DirectiveVerb::Ask, "architect", &budget);
        assert!(result.contains("pm"));
        assert!(result.contains("ask"));
        assert!(result.contains("architect"));
        assert!(result.contains("[10/10]"));
    }

    #[test]
    fn format_routing_status_handover() {
        let budget = Budget::new(5);
        budget.try_consume();
        let result = format_routing_status("pm", &DirectiveVerb::Handover, "deployer", &budget);
        assert!(result.contains("handover"));
        assert!(result.contains("[4/5]"));
    }

    #[test]
    fn format_routing_status_contains_glyph() {
        let budget = Budget::new(10);
        let result = format_routing_status("pm", &DirectiveVerb::Ask, "architect", &budget);
        assert!(result.contains('▶'));
    }

    #[test]
    fn format_routing_status_no_payload() {
        let budget = Budget::new(10);
        let result = format_routing_status("pm", &DirectiveVerb::Ask, "architect", &budget);
        // Should not contain payload or quotes
        assert!(!result.contains('"'));
    }

    // =========================================================================
    // format_directive_banner tests
    // =========================================================================

    #[test]
    fn format_directive_banner_contains_payload() {
        let result = format_directive_banner("Please review this code", 80);
        assert!(result.contains("review"));
    }

    #[test]
    fn format_directive_banner_has_separators() {
        let result = format_directive_banner("short", 80);
        assert!(result.contains('─'));
    }

    #[test]
    fn format_directive_banner_separator_width_matches_minimum() {
        let result = format_directive_banner("hi", 80);
        // The separator should be 40 chars (MIN_SEPARATOR_WIDTH)
        let line = result.lines().next().unwrap();
        // Strip ANSI codes to count visible chars
        let visible: String = line.replace("\x1b[2m", "").replace("\x1b[0m", "");
        assert_eq!(visible.chars().count(), 40);
    }

    // =========================================================================
    // format_orchestration_summary tests
    // =========================================================================

    #[test]
    fn format_orchestration_summary_shows_budget() {
        let budget = Budget::new(10);
        budget.try_consume();
        budget.try_consume();
        budget.try_consume();
        let result = format_orchestration_summary(&budget);
        assert!(result.contains('✓'));
        assert!(result.contains("[7/10]"));
    }

    #[test]
    fn format_orchestration_summary_full_budget() {
        let budget = Budget::new(10);
        let result = format_orchestration_summary(&budget);
        assert!(result.contains("[10/10]"));
    }

    // =========================================================================
    // format_persona_banner tests
    // =========================================================================

    #[test]
    fn format_persona_banner_contains_name() {
        let result = format_persona_banner("architect");
        assert!(result.contains("architect"));
    }

    #[test]
    fn format_persona_banner_contains_box_drawing() {
        let result = format_persona_banner("pm");
        assert!(result.contains('━'));
    }

    #[test]
    fn format_persona_banner_contains_ansi_codes() {
        let result = format_persona_banner("reviewer");
        assert!(result.contains("\x1b["));
    }
}
