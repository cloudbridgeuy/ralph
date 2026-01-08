//! Pure formatting functions for display values.

/// Format token count with thousands separators for readability.
pub(super) fn format_token_count(tokens: u64) -> String {
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
