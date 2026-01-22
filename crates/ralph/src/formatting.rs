//! Shared pure formatting functions (Functional Core).
//!
//! This module contains stateless formatting utilities used across
//! multiple modules. All functions are pure with no side effects.

/// Format duration from milliseconds to human-readable string.
///
/// # Formatting rules
/// - 0-999ms -> "Xms"
/// - 1000-59999ms -> "X.Xs" (e.g., "45.2s")
/// - 60000+ ms -> "Xm Ys" (e.g., "1m 23s")
///
/// # Examples
///
/// ```
/// use ralph::formatting::format_duration;
///
/// assert_eq!(format_duration(500), "500ms");
/// assert_eq!(format_duration(1500), "1.5s");
/// assert_eq!(format_duration(125000), "2m 5s");
/// ```
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let minutes = ms / 60_000;
        let seconds = (ms % 60_000) / 1000;
        format!("{}m {}s", minutes, seconds)
    }
}

/// Format token count with thousands suffix for readability.
///
/// # Examples
///
/// ```
/// use ralph::formatting::format_token_count;
///
/// assert_eq!(format_token_count(500), "500");
/// assert_eq!(format_token_count(1500), "1.5K");
/// assert_eq!(format_token_count(1_500_000), "1.50M");
/// ```
pub fn format_token_count(tokens: u64) -> String {
    if tokens < 1000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}K", tokens as f64 / 1000.0)
    } else {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration_milliseconds() {
        assert_eq!(format_duration(0), "0ms");
        assert_eq!(format_duration(500), "500ms");
        assert_eq!(format_duration(999), "999ms");
    }

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(1000), "1.0s");
        assert_eq!(format_duration(1500), "1.5s");
        assert_eq!(format_duration(45200), "45.2s");
        assert_eq!(format_duration(59999), "60.0s");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(60_000), "1m 0s");
        assert_eq!(format_duration(125_000), "2m 5s");
        assert_eq!(format_duration(3_723_000), "62m 3s");
    }

    #[test]
    fn test_format_token_count_under_thousand() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(999), "999");
    }

    #[test]
    fn test_format_token_count_thousands() {
        assert_eq!(format_token_count(1000), "1.0K");
        assert_eq!(format_token_count(1500), "1.5K");
        assert_eq!(format_token_count(999_999), "1000.0K");
    }

    #[test]
    fn test_format_token_count_millions() {
        assert_eq!(format_token_count(1_000_000), "1.00M");
        assert_eq!(format_token_count(1_500_000), "1.50M");
    }
}
