//! Sessions display with detailed metadata (Functional Core + Imperative Shell).
//!
//! This module provides rich session listing with aggregated metadata from
//! iteration logs, including cost, duration, and token counts.
//!
//! # Data Flow
//!
//! 1. Load sessions from global index (SessionEntry)
//! 2. For each session, load iteration logs from session directory
//! 3. Aggregate metadata (cost, duration, tokens) from iteration logs
//! 4. Display in formatted table
//!
//! # Example
//!
//! ```no_run
//! use ralph::sessions_display::{list_sessions, SessionsFilter};
//!
//! // List all sessions with detailed metadata
//! list_sessions(SessionsFilter::default()).unwrap();
//! ```

use crate::formatting::format_duration;
use crate::iteration::IterationLog;
use crate::paths;
use crate::session::load_sessions_index;
use chrono::{DateTime, Local};
use ralph_core::session::{SessionEntry, SessionOutcome};
use std::fs;
use std::io::IsTerminal;

/// Error type for sessions display operations.
#[derive(Debug, thiserror::Error)]
pub enum SessionsDisplayError {
    /// Failed to load sessions index.
    #[error("Failed to load sessions index: {0}")]
    LoadSessionsIndex(#[from] crate::session::SessionError),
}

/// Filter criteria for listing sessions.
#[derive(Debug, Default, Clone)]
pub struct SessionsFilter {
    /// Filter by project path (substring match).
    pub project: Option<String>,
    /// Filter by outcome status.
    pub outcome: Option<String>,
}

// =============================================================================
// Functional Core: Pure aggregation and formatting functions
// =============================================================================

/// Aggregated session statistics from iteration logs.
#[derive(Debug, Default)]
pub struct SessionStats {
    /// Total cost in USD across all iterations.
    pub total_cost_usd: Option<f64>,
    /// Total duration in milliseconds across all iterations.
    pub total_duration_ms: Option<u64>,
    /// Total input tokens across all iterations.
    pub input_tokens: Option<u64>,
    /// Total output tokens across all iterations.
    pub output_tokens: Option<u64>,
}

/// A session entry with aggregated statistics.
#[derive(Debug)]
pub struct SessionWithStats {
    /// The session entry from the global index.
    pub entry: SessionEntry,
    /// Aggregated statistics from iteration logs.
    pub stats: SessionStats,
}

/// Aggregate statistics from a list of iteration logs (pure function).
///
/// # Arguments
///
/// * `logs` - Iteration logs to aggregate
///
/// # Returns
///
/// Aggregated statistics with totals for cost, duration, and tokens.
pub fn aggregate_iteration_stats(logs: &[IterationLog]) -> SessionStats {
    let mut stats = SessionStats::default();

    for log in logs {
        if let Some(ref metadata) = log.metadata {
            if let Some(cost) = metadata.cost_usd {
                *stats.total_cost_usd.get_or_insert(0.0) += cost;
            }
            if let Some(duration) = metadata.duration_ms {
                *stats.total_duration_ms.get_or_insert(0) += duration;
            }
            if let Some(ref usage) = metadata.usage {
                *stats.input_tokens.get_or_insert(0) += usage.input_tokens;
                *stats.output_tokens.get_or_insert(0) += usage.output_tokens;
            }
        }
    }

    stats
}

/// Aggregate totals from a list of session stats (pure function).
///
/// Computes combined totals across multiple sessions.
pub fn aggregate_session_totals(sessions: &[SessionWithStats]) -> SessionStats {
    let mut totals = SessionStats::default();

    for session in sessions {
        if let Some(cost) = session.stats.total_cost_usd {
            *totals.total_cost_usd.get_or_insert(0.0) += cost;
        }
        if let Some(duration) = session.stats.total_duration_ms {
            *totals.total_duration_ms.get_or_insert(0) += duration;
        }
        if let Some(input) = session.stats.input_tokens {
            *totals.input_tokens.get_or_insert(0) += input;
        }
        if let Some(output) = session.stats.output_tokens {
            *totals.output_tokens.get_or_insert(0) += output;
        }
    }

    totals
}

/// Format token count to a human-readable string (pure function).
///
/// Uses K suffix for thousands.
///
/// # Examples
///
/// - 500 -> "500"
/// - 1500 -> "1.5K"
/// - 150000 -> "150K"
pub fn format_tokens(count: u64) -> String {
    if count < 1000 {
        count.to_string()
    } else {
        format!("{:.1}K", count as f64 / 1000.0)
    }
}

/// Format cost in USD (pure function).
///
/// # Examples
///
/// - 0.1234 -> "$0.12"
/// - 1.5 -> "$1.50"
pub fn format_cost(cost: f64) -> String {
    format!("${:.2}", cost)
}

/// Truncate a string from the left with "..." prefix (pure function).
///
/// Uses character-based iteration to safely handle Unicode strings.
pub fn truncate_left(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count > max_len {
        let skip = char_count - (max_len - 3);
        format!("...{}", s.chars().skip(skip).collect::<String>())
    } else {
        s.to_string()
    }
}

// =============================================================================
// Imperative Shell: I/O operations and display
// =============================================================================

/// Load iteration logs for a session from disk.
fn load_session_iteration_logs(slug: &str) -> Vec<IterationLog> {
    let session_dir = paths::session_dir(slug);

    if !session_dir.exists() {
        return Vec::new();
    }

    let entries = match fs::read_dir(&session_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut logs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("iteration-") && name.ends_with(".toml") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(log) = toml::from_str::<IterationLog>(&content) {
                        logs.push(log);
                    }
                }
            }
        }
    }

    logs
}

/// Check if a session matches the filter criteria (pure function).
///
/// Enables independent unit testing of filter logic.
fn matches_filter(session: &SessionEntry, filter: &SessionsFilter) -> bool {
    if let Some(ref project_filter) = filter.project {
        if !session
            .project
            .display()
            .to_string()
            .contains(project_filter)
        {
            return false;
        }
    }
    if let Some(ref outcome_filter) = filter.outcome {
        let outcome_str = session.outcome.to_string();
        if !outcome_str.eq_ignore_ascii_case(outcome_filter) {
            return false;
        }
    }
    true
}

/// Load all sessions with their aggregated statistics.
fn load_sessions_with_stats(
    filter: &SessionsFilter,
) -> Result<Vec<SessionWithStats>, SessionsDisplayError> {
    let index = load_sessions_index()?;

    let mut sessions: Vec<SessionWithStats> = index
        .sessions
        .into_iter()
        .filter(|s| matches_filter(s, filter))
        .map(|entry| {
            let logs = load_session_iteration_logs(&entry.slug);
            let stats = aggregate_iteration_stats(&logs);
            SessionWithStats { entry, stats }
        })
        .collect();

    // Sort by date (most recent first)
    sessions.sort_by(|a, b| b.entry.started_at.cmp(&a.entry.started_at));

    Ok(sessions)
}

/// List sessions with detailed metadata.
///
/// Displays a formatted table with session information including
/// cost, duration, and token counts aggregated from iteration logs.
pub fn list_sessions(filter: SessionsFilter) -> Result<(), SessionsDisplayError> {
    let sessions = load_sessions_with_stats(&filter)?;

    if sessions.is_empty() {
        if filter.project.is_some() || filter.outcome.is_some() {
            println!("No sessions found matching the specified filters.");
        } else {
            println!("No sessions found. Run 'ralph run' to start a session.");
        }
        return Ok(());
    }

    // Check if any session has cost data
    let has_cost_data = sessions.iter().any(|s| s.stats.total_cost_usd.is_some());
    let has_token_data = sessions.iter().any(|s| s.stats.input_tokens.is_some());

    let is_terminal = std::io::stdout().is_terminal();

    // Display table
    display_sessions_table(&sessions, has_cost_data, has_token_data, is_terminal);

    // Calculate and display summary
    let totals = aggregate_session_totals(&sessions);
    display_summary(sessions.len(), &totals);

    Ok(())
}

/// Display summary line with totals (imperative shell).
fn display_summary(session_count: usize, totals: &SessionStats) {
    println!();
    let mut summary_parts = vec![format!("{} session(s)", session_count)];

    if let Some(cost) = totals.total_cost_usd {
        summary_parts.push(format!("Total cost: {}", format_cost(cost)));
    }
    if let Some(duration) = totals.total_duration_ms {
        summary_parts.push(format!("Total duration: {}", format_duration(duration)));
    }
    if let (Some(input), Some(output)) = (totals.input_tokens, totals.output_tokens) {
        summary_parts.push(format!(
            "Tokens: {} in / {} out",
            format_tokens(input),
            format_tokens(output)
        ));
    }

    println!("{}", summary_parts.join(" | "));
}

/// Display sessions in a formatted table.
fn display_sessions_table(
    sessions: &[SessionWithStats],
    has_cost_data: bool,
    has_token_data: bool,
    is_terminal: bool,
) {
    // Define column widths
    let slug_width = 18;
    let project_width = if has_cost_data || has_token_data {
        25
    } else {
        35
    };
    let date_width = 16;
    let iters_width = 5;
    let duration_width = 8;
    let cost_width = 8;
    let tokens_width = 12;
    let outcome_width = 11;

    // Build header based on available data
    let mut header = format!(
        "{:<slug_width$} {:<project_width$} {:<date_width$} {:>iters_width$}",
        "SLUG", "PROJECT", "DATE", "ITERS",
    );

    let mut sep_len = slug_width + project_width + date_width + iters_width + 3;

    if has_cost_data {
        header.push_str(&format!(
            " {:>duration_width$} {:>cost_width$}",
            "DURATION", "COST"
        ));
        sep_len += duration_width + cost_width + 2;
    }

    if has_token_data {
        header.push_str(&format!(" {:>tokens_width$}", "TOKENS"));
        sep_len += tokens_width + 1;
    }

    header.push_str(&format!(" {:<outcome_width$}", "OUTCOME"));
    sep_len += outcome_width + 1;

    println!("{}", header);
    println!("{}", "-".repeat(sep_len));

    // Print sessions
    for session in sessions {
        let entry = &session.entry;
        let stats = &session.stats;

        // Format project path (truncate if needed)
        let project_str = entry.project.display().to_string();
        let project_display = truncate_left(&project_str, project_width - 2);

        // Format date (local time)
        let local_time: DateTime<Local> = entry.started_at.with_timezone(&Local);
        let date_str = local_time.format("%Y-%m-%d %H:%M").to_string();

        // Format outcome with color if terminal
        let outcome_str = format_outcome(entry.outcome, is_terminal);

        // Build row
        let mut row = format!(
            "{:<slug_width$} {:<project_width$} {:<date_width$} {:>iters_width$}",
            entry.slug, project_display, date_str, entry.iterations,
        );

        if has_cost_data {
            let duration_str = stats
                .total_duration_ms
                .map(format_duration)
                .unwrap_or_else(|| "-".to_string());
            let cost_str = stats
                .total_cost_usd
                .map(format_cost)
                .unwrap_or_else(|| "-".to_string());
            row.push_str(&format!(
                " {:>duration_width$} {:>cost_width$}",
                duration_str, cost_str
            ));
        }

        if has_token_data {
            let tokens_str = match (stats.input_tokens, stats.output_tokens) {
                (Some(input), Some(output)) => {
                    format!("{}/{}", format_tokens(input), format_tokens(output))
                }
                _ => "-".to_string(),
            };
            row.push_str(&format!(" {:>tokens_width$}", tokens_str));
        }

        row.push_str(&format!(" {:<outcome_width$}", outcome_str));

        println!("{}", row);

        // Print replay hint
        println!(
            "{}  -> ralph replay {}",
            " ".repeat(slug_width - 2),
            entry.slug
        );
    }
}

/// Format session outcome with optional color (pure function).
fn format_outcome(outcome: SessionOutcome, is_terminal: bool) -> String {
    let outcome_str = match outcome {
        SessionOutcome::Completed => "completed",
        SessionOutcome::InProgress => "in_progress",
        SessionOutcome::Aborted => "aborted",
        SessionOutcome::Failed => "failed",
        SessionOutcome::Interrupted => "interrupted",
    };

    if is_terminal {
        match outcome {
            SessionOutcome::Completed => format!("\x1b[32m{}\x1b[0m", outcome_str), // Green
            SessionOutcome::Failed => format!("\x1b[31m{}\x1b[0m", outcome_str),    // Red
            SessionOutcome::Aborted => format!("\x1b[33m{}\x1b[0m", outcome_str),   // Yellow
            SessionOutcome::Interrupted => format!("\x1b[33m{}\x1b[0m", outcome_str), // Yellow
            SessionOutcome::InProgress => format!("\x1b[36m{}\x1b[0m", outcome_str), // Cyan
        }
    } else {
        outcome_str.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iteration::LogMetadata;
    use ralph_core::stream::Usage;

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
        assert_eq!(format_duration(59999), "60.0s");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(60000), "1m 0s");
        assert_eq!(format_duration(125000), "2m 5s");
        assert_eq!(format_duration(3661000), "61m 1s");
    }

    #[test]
    fn test_format_tokens_small() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(999), "999");
    }

    #[test]
    fn test_format_tokens_thousands() {
        assert_eq!(format_tokens(1000), "1.0K");
        assert_eq!(format_tokens(1500), "1.5K");
        assert_eq!(format_tokens(150000), "150.0K");
    }

    #[test]
    fn test_format_cost() {
        assert_eq!(format_cost(0.0), "$0.00");
        assert_eq!(format_cost(0.1234), "$0.12");
        assert_eq!(format_cost(1.5), "$1.50");
        assert_eq!(format_cost(99.999), "$100.00");
    }

    #[test]
    fn test_truncate_left() {
        assert_eq!(truncate_left("short", 10), "short");
        assert_eq!(truncate_left("exactly10!", 10), "exactly10!");
        assert_eq!(truncate_left("this is too long", 10), "...oo long");
    }

    #[test]
    fn test_aggregate_iteration_stats_empty() {
        let logs: Vec<IterationLog> = vec![];
        let stats = aggregate_iteration_stats(&logs);

        assert!(stats.total_cost_usd.is_none());
        assert!(stats.total_duration_ms.is_none());
        assert!(stats.input_tokens.is_none());
        assert!(stats.output_tokens.is_none());
    }

    #[test]
    fn test_aggregate_iteration_stats_with_data() {
        let logs = vec![
            create_test_log(Some(0.10), Some(1000), Some(100), Some(50)),
            create_test_log(Some(0.20), Some(2000), Some(200), Some(100)),
            create_test_log(None, None, None, None),
        ];

        let stats = aggregate_iteration_stats(&logs);

        // Use approximate comparison for floats
        assert!((stats.total_cost_usd.unwrap() - 0.30).abs() < 0.0001);
        assert_eq!(stats.total_duration_ms, Some(3000));
        assert_eq!(stats.input_tokens, Some(300));
        assert_eq!(stats.output_tokens, Some(150));
    }

    #[test]
    fn test_aggregate_iteration_stats_partial_data() {
        // Note: Usage struct requires both input and output tokens,
        // so we can only test partial cost and duration data.
        let logs = vec![
            create_test_log(Some(0.10), None, None, None),
            create_test_log(None, Some(2000), Some(100), Some(50)),
        ];

        let stats = aggregate_iteration_stats(&logs);

        // Use approximate comparison for floats
        assert!((stats.total_cost_usd.unwrap() - 0.10).abs() < 0.0001);
        assert_eq!(stats.total_duration_ms, Some(2000));
        assert_eq!(stats.input_tokens, Some(100));
        assert_eq!(stats.output_tokens, Some(50));
    }

    #[test]
    fn test_format_outcome_plain() {
        assert_eq!(
            format_outcome(SessionOutcome::Completed, false),
            "completed"
        );
        assert_eq!(format_outcome(SessionOutcome::Failed, false), "failed");
        assert_eq!(
            format_outcome(SessionOutcome::InProgress, false),
            "in_progress"
        );
    }

    #[test]
    fn test_format_outcome_terminal_contains_color() {
        let completed = format_outcome(SessionOutcome::Completed, true);
        assert!(completed.contains("\x1b[32m")); // Green
        assert!(completed.contains("completed"));

        let failed = format_outcome(SessionOutcome::Failed, true);
        assert!(failed.contains("\x1b[31m")); // Red
        assert!(failed.contains("failed"));
    }

    #[test]
    fn test_truncate_left_unicode() {
        // Test with Unicode characters to ensure UTF-8 safety
        let unicode_path = "/Users/日本語/プロジェクト";
        let truncated = truncate_left(unicode_path, 15);
        assert!(truncated.starts_with("..."));
        assert_eq!(truncated.chars().count(), 15);
    }

    #[test]
    fn test_matches_filter_no_filter() {
        let filter = SessionsFilter::default();
        let entry =
            create_test_session_entry("test-slug", "/test/project", SessionOutcome::Completed);
        assert!(matches_filter(&entry, &filter));
    }

    #[test]
    fn test_matches_filter_project_match() {
        let filter = SessionsFilter {
            project: Some("project".to_string()),
            outcome: None,
        };
        let entry =
            create_test_session_entry("test-slug", "/test/project", SessionOutcome::Completed);
        assert!(matches_filter(&entry, &filter));
    }

    #[test]
    fn test_matches_filter_project_no_match() {
        let filter = SessionsFilter {
            project: Some("other".to_string()),
            outcome: None,
        };
        let entry =
            create_test_session_entry("test-slug", "/test/project", SessionOutcome::Completed);
        assert!(!matches_filter(&entry, &filter));
    }

    #[test]
    fn test_matches_filter_outcome_match() {
        let filter = SessionsFilter {
            project: None,
            outcome: Some("completed".to_string()),
        };
        let entry =
            create_test_session_entry("test-slug", "/test/project", SessionOutcome::Completed);
        assert!(matches_filter(&entry, &filter));
    }

    #[test]
    fn test_matches_filter_outcome_no_match() {
        let filter = SessionsFilter {
            project: None,
            outcome: Some("failed".to_string()),
        };
        let entry =
            create_test_session_entry("test-slug", "/test/project", SessionOutcome::Completed);
        assert!(!matches_filter(&entry, &filter));
    }

    #[test]
    fn test_aggregate_session_totals() {
        let sessions = vec![
            create_test_session_with_stats(Some(0.10), Some(1000), Some(100), Some(50)),
            create_test_session_with_stats(Some(0.20), Some(2000), Some(200), Some(100)),
            create_test_session_with_stats(None, None, None, None),
        ];

        let totals = aggregate_session_totals(&sessions);

        assert!((totals.total_cost_usd.unwrap() - 0.30).abs() < 0.0001);
        assert_eq!(totals.total_duration_ms, Some(3000));
        assert_eq!(totals.input_tokens, Some(300));
        assert_eq!(totals.output_tokens, Some(150));
    }

    // Helper to create test session entry
    fn create_test_session_entry(
        slug: &str,
        project: &str,
        outcome: SessionOutcome,
    ) -> SessionEntry {
        SessionEntry {
            slug: slug.to_string(),
            project: std::path::PathBuf::from(project),
            started_at: chrono::Utc::now(),
            completed_at: None,
            iterations: 1,
            outcome,
        }
    }

    // Helper to create test session with stats
    fn create_test_session_with_stats(
        cost: Option<f64>,
        duration: Option<u64>,
        input: Option<u64>,
        output: Option<u64>,
    ) -> SessionWithStats {
        SessionWithStats {
            entry: create_test_session_entry("test", "/test", SessionOutcome::Completed),
            stats: SessionStats {
                total_cost_usd: cost,
                total_duration_ms: duration,
                input_tokens: input,
                output_tokens: output,
            },
        }
    }

    // Helper to create test iteration logs
    fn create_test_log(
        cost: Option<f64>,
        duration: Option<u64>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> IterationLog {
        let usage = match (input_tokens, output_tokens) {
            (Some(input), Some(output)) => Some(Usage {
                input_tokens: input,
                output_tokens: output,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            }),
            _ => None,
        };

        let metadata = if cost.is_some() || duration.is_some() || usage.is_some() {
            Some(LogMetadata {
                cost_usd: cost,
                duration_ms: duration,
                usage,
                ..Default::default()
            })
        } else {
            None
        };

        IterationLog {
            sequence: 1,
            started_at: chrono::Utc::now(),
            completed_at: chrono::Utc::now(),
            exit_code: 0,
            pending_before: 0,
            pending_after: 0,
            prompt: None,
            response: None,
            metadata,
            tool_calls: vec![],
            chunks: vec![],
            output_blocks: vec![],
        }
    }
}
