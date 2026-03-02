//! Display functions for orchestration routing status and summaries.

use super::Budget;
use ralph_core::directive::DirectiveVerb;

/// Print a routing status line showing the directive being executed.
///
/// Displays the originator persona, verb, target persona, a preview of the
/// payload, and the current budget state.
pub fn print_routing_status(
    originator: &str,
    verb: &DirectiveVerb,
    target: &str,
    payload_preview: &str,
    budget: &Budget,
) {
    let verb_str = match verb {
        DirectiveVerb::Ask => "ask",
        DirectiveVerb::Handover => "handover",
    };

    // Truncate payload preview for display (UTF-8 safe)
    let preview = truncate_preview(payload_preview, 80);

    println!("───── ralph: routing ─────");
    println!(
        "{} → {} → {}: \"{}\"",
        originator, verb_str, target, preview
    );
    println!(
        "Budget: {}/{} remaining",
        budget.remaining(),
        budget.limit()
    );
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

/// Print an orchestration completion summary showing budget usage.
pub fn print_orchestration_summary(budget: &Budget) {
    let used = budget.limit() - budget.remaining();
    println!(
        "───── orchestration complete: {}/{} invocations used ─────",
        used,
        budget.limit()
    );
}
