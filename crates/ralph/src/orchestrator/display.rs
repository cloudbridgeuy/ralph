//! Display functions for orchestration routing status and summaries.
//!
//! Follows Functional Core - Imperative Shell: `format_*` functions are pure
//! (return `String`), `print_*` functions handle I/O.

use super::Budget;
use crate::ansi::{CYAN, DIM, GREEN, RESET};
use ralph_core::directive::DirectiveVerb;

/// Format a routing status line showing the directive being executed.
///
/// Pure function — returns a styled string with ANSI codes.
///
/// # Format
///
/// ```text
/// ▶ pm → ask → architect                                    [9/10]
///   "What are the technical..."
/// ```
fn format_routing_status(
    originator: &str,
    verb: &DirectiveVerb,
    target: &str,
    payload_preview: &str,
    budget: &Budget,
) -> String {
    let verb_str = match verb {
        DirectiveVerb::Ask => "ask",
        DirectiveVerb::Handover => "handover",
    };

    let preview = truncate_preview(payload_preview, 80);

    let header =
        format!("{CYAN}▶{RESET} {CYAN}{originator}{RESET} → {verb_str} → {CYAN}{target}{RESET}");
    let budget_display = format!("{DIM}[{}/{}]{RESET}", budget.remaining(), budget.limit());
    let payload_line = format!("{DIM}  \"{preview}\"{RESET}");

    format!("{header}  {budget_display}\n{payload_line}")
}

/// Print a routing status line to stdout.
///
/// Imperative shell wrapper around [`format_routing_status`].
pub fn print_routing_status(
    originator: &str,
    verb: &DirectiveVerb,
    target: &str,
    payload_preview: &str,
    budget: &Budget,
) {
    println!(
        "{}",
        format_routing_status(originator, verb, target, payload_preview, budget)
    );
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

/// Truncate a string to a maximum number of characters, appending "..." if truncated.
///
/// Uses `char_indices` to avoid panicking on multi-byte UTF-8 boundaries.
fn truncate_preview(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        return s.to_string();
    }

    let truncate_to = max_chars.saturating_sub(3);
    let end_byte = s
        .char_indices()
        .nth(truncate_to)
        .map_or(s.len(), |(idx, _)| idx);
    format!("{}...", &s[..end_byte])
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // truncate_preview tests
    // =========================================================================

    #[test]
    fn truncate_preview_short_string() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn truncate_preview_exact_length() {
        assert_eq!(truncate_preview("hello", 5), "hello");
    }

    #[test]
    fn truncate_preview_long_string() {
        let result = truncate_preview("hello world, this is long", 10);
        assert!(result.ends_with("..."));
        assert!(result.chars().count() <= 10);
    }

    #[test]
    fn truncate_preview_multibyte() {
        let result = truncate_preview("héllo wörld café", 8);
        assert!(result.ends_with("..."));
    }

    // =========================================================================
    // format_routing_status tests
    // =========================================================================

    #[test]
    fn format_routing_status_ask() {
        let budget = Budget::new(10);
        let result = format_routing_status(
            "pm",
            &DirectiveVerb::Ask,
            "architect",
            "What are the responsibilities?",
            &budget,
        );
        assert!(result.contains("pm"));
        assert!(result.contains("ask"));
        assert!(result.contains("architect"));
        assert!(result.contains("What are the responsibilities?"));
        assert!(result.contains("[10/10]"));
    }

    #[test]
    fn format_routing_status_handover() {
        let budget = Budget::new(5);
        budget.try_consume();
        let result = format_routing_status(
            "pm",
            &DirectiveVerb::Handover,
            "deployer",
            "Deploy to prod",
            &budget,
        );
        assert!(result.contains("handover"));
        assert!(result.contains("[4/5]"));
    }

    #[test]
    fn format_routing_status_truncates_long_payload() {
        let budget = Budget::new(10);
        let long_payload = "a".repeat(100);
        let result = format_routing_status(
            "pm",
            &DirectiveVerb::Ask,
            "architect",
            &long_payload,
            &budget,
        );
        assert!(result.contains("..."));
        assert!(!result.contains(&"a".repeat(100)));
    }

    #[test]
    fn format_routing_status_contains_glyph() {
        let budget = Budget::new(10);
        let result =
            format_routing_status("pm", &DirectiveVerb::Ask, "architect", "hello", &budget);
        assert!(result.contains('▶'));
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
}
