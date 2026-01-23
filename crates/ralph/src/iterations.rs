//! Iterations listing functionality (Imperative Shell).
//!
//! This module provides the ability to list all iterations across all sessions,
//! with filtering by session, project, and outcome. It reads iteration logs
//! from session directories and displays them in a table format.
//!
//! # Example
//!
//! ```no_run
//! use ralph::iterations::{list_iterations, IterationsFilter};
//!
//! // List all iterations
//! list_iterations(IterationsFilter::default()).unwrap();
//!
//! // Filter by session
//! list_iterations(IterationsFilter {
//!     session: Some("quiet-mountain".to_string()),
//!     ..Default::default()
//! }).unwrap();
//! ```

use crate::formatting::format_duration;
use crate::iteration::IterationLog;
use crate::paths;
use crate::session::load_sessions_index;
use chrono::{DateTime, Local};
use ralph_core::session::SessionEntry;
use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;

/// Error type for iterations listing operations.
#[derive(Debug, thiserror::Error)]
pub enum IterationsError {
    /// Failed to load sessions index.
    #[error("Failed to load sessions index: {0}")]
    LoadSessionsIndex(#[from] crate::session::SessionError),

    /// Failed to read session directory.
    #[error("Failed to read session directory at {path}: {source}")]
    ReadSessionDir {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to read iteration log.
    #[error("Failed to read iteration log at {path}: {source}")]
    ReadIterationLog {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to parse iteration log.
    #[error("Failed to parse iteration log at {path}: {source}")]
    ParseIterationLog {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    /// Invalid outcome filter value.
    #[error("Invalid outcome filter '{value}'. Valid values: completed, failed")]
    InvalidOutcomeFilter { value: String },
}

/// Filter criteria for listing iterations.
#[derive(Debug, Default, Clone)]
pub struct IterationsFilter {
    /// Filter by session slug.
    pub session: Option<String>,
    /// Filter by project path (substring match).
    pub project: Option<String>,
    /// Filter by outcome: "completed" (exit_code 0) or "failed" (non-zero exit_code).
    pub outcome: Option<String>,
}

/// A single iteration entry with its associated session information.
#[derive(Debug)]
pub struct IterationEntry {
    /// Session slug this iteration belongs to.
    pub session_slug: String,
    /// Project path from the session.
    pub project: PathBuf,
    /// The iteration log data.
    pub log: IterationLog,
}

/// Result of listing iterations.
pub struct IterationsResult {
    /// Number of iterations listed.
    pub count: usize,
    /// Total cost across all listed iterations (if available).
    pub total_cost_usd: Option<f64>,
    /// Total duration across all listed iterations (if available).
    pub total_duration_ms: Option<u64>,
}

/// List all iterations matching the given filter.
///
/// Reads all iteration logs from all session directories and displays them
/// in a table format, sorted by start time (most recent first).
///
/// # Arguments
///
/// * `filter` - Filter criteria for which iterations to include
///
/// # Returns
///
/// * `Ok(IterationsResult)` - Summary of the listed iterations
/// * `Err(IterationsError)` - If loading or parsing fails
pub fn list_iterations(filter: IterationsFilter) -> Result<IterationsResult, IterationsError> {
    // Validate outcome filter if provided
    if let Some(ref outcome) = filter.outcome {
        let normalized = outcome.to_lowercase();
        if normalized != "completed" && normalized != "failed" {
            return Err(IterationsError::InvalidOutcomeFilter {
                value: outcome.clone(),
            });
        }
    }

    // Load all iterations
    let iterations = load_all_iterations(&filter)?;

    if iterations.is_empty() {
        if filter.session.is_some() || filter.project.is_some() || filter.outcome.is_some() {
            println!("No iterations found matching the specified filters.");
        } else {
            println!("No iterations found. Run 'ralph run' to start a session.");
        }
        return Ok(IterationsResult {
            count: 0,
            total_cost_usd: None,
            total_duration_ms: None,
        });
    }

    // Calculate totals
    let mut total_cost: Option<f64> = None;
    let mut total_duration: Option<u64> = None;

    for entry in &iterations {
        if let Some(ref metadata) = entry.log.metadata {
            if let Some(cost) = metadata.cost_usd {
                *total_cost.get_or_insert(0.0) += cost;
            }
            if let Some(duration) = metadata.duration_ms {
                *total_duration.get_or_insert(0) += duration;
            }
        }
    }

    // Display table
    display_iterations_table(&iterations, total_cost, total_duration);

    Ok(IterationsResult {
        count: iterations.len(),
        total_cost_usd: total_cost,
        total_duration_ms: total_duration,
    })
}

/// Load all iteration entries from all sessions, applying filters.
fn load_all_iterations(filter: &IterationsFilter) -> Result<Vec<IterationEntry>, IterationsError> {
    let index = load_sessions_index()?;

    let mut all_iterations: Vec<IterationEntry> = Vec::new();

    for session in &index.sessions {
        // Apply session filter
        if let Some(ref session_filter) = filter.session {
            if session.slug != *session_filter {
                continue;
            }
        }

        // Apply project filter
        if let Some(ref project_filter) = filter.project {
            let project_str = session.project.display().to_string();
            if !project_str.contains(project_filter) {
                continue;
            }
        }

        // Load iterations for this session
        let session_iterations = load_session_iterations(session, filter)?;
        all_iterations.extend(session_iterations);
    }

    // Sort by started_at descending (most recent first)
    all_iterations.sort_by(|a, b| b.log.started_at.cmp(&a.log.started_at));

    Ok(all_iterations)
}

/// Load iterations for a single session.
fn load_session_iterations(
    session: &SessionEntry,
    filter: &IterationsFilter,
) -> Result<Vec<IterationEntry>, IterationsError> {
    let session_dir = paths::session_dir(&session.slug);

    if !session_dir.exists() {
        // Session directory doesn't exist - skip silently
        return Ok(Vec::new());
    }

    let entries = match fs::read_dir(&session_dir) {
        Ok(entries) => entries,
        Err(e) => {
            // Log warning but don't fail - session might have been deleted
            eprintln!(
                "Warning: Could not read session directory {}: {}",
                session_dir.display(),
                e
            );
            return Ok(Vec::new());
        }
    };

    let mut iterations = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Match iteration-N.toml files
            if name.starts_with("iteration-") && name.ends_with(".toml") {
                if let Ok(log) = load_iteration_log(&path) {
                    // Apply outcome filter
                    if let Some(ref outcome_filter) = filter.outcome {
                        let normalized = outcome_filter.to_lowercase();
                        let is_completed = log.exit_code == 0;
                        if normalized == "completed" && !is_completed {
                            continue;
                        }
                        if normalized == "failed" && is_completed {
                            continue;
                        }
                    }

                    iterations.push(IterationEntry {
                        session_slug: session.slug.clone(),
                        project: session.project.clone(),
                        log,
                    });
                }
            }
        }
    }

    Ok(iterations)
}

/// Load and parse a single iteration log file.
fn load_iteration_log(path: &PathBuf) -> Result<IterationLog, IterationsError> {
    let content = fs::read_to_string(path).map_err(|e| IterationsError::ReadIterationLog {
        path: path.display().to_string(),
        source: e,
    })?;

    toml::from_str(&content).map_err(|e| IterationsError::ParseIterationLog {
        path: path.display().to_string(),
        source: e,
    })
}

/// Display iterations in a table format.
fn display_iterations_table(
    iterations: &[IterationEntry],
    total_cost: Option<f64>,
    total_duration: Option<u64>,
) {
    let is_terminal = std::io::stdout().is_terminal();

    // Check if any iteration has cost data to determine whether to show cost column
    let has_cost_data = iterations
        .iter()
        .any(|e| e.log.metadata.as_ref().and_then(|m| m.cost_usd).is_some());

    // Print header
    if has_cost_data {
        println!(
            "{:<20} {:<4} {:<35} {:<17} {:<10} {:<5} {:<10}",
            "SESSION", "#", "PROJECT", "STARTED", "DURATION", "EXIT", "COST"
        );
        println!("{}", "-".repeat(105));
    } else {
        println!(
            "{:<20} {:<4} {:<40} {:<17} {:<10} {:<5}",
            "SESSION", "#", "PROJECT", "STARTED", "DURATION", "EXIT"
        );
        println!("{}", "-".repeat(98));
    }

    // Print iterations
    for entry in iterations {
        // Truncate project path if too long
        let project_str = entry.project.display().to_string();
        let max_project_len = if has_cost_data { 33 } else { 38 };
        let project_display = if project_str.len() > max_project_len {
            format!(
                "...{}",
                &project_str[project_str.len() - (max_project_len - 3)..]
            )
        } else {
            project_str
        };

        // Format timestamp to local time
        let local_time: DateTime<Local> = entry.log.started_at.with_timezone(&Local);
        let date_str = local_time.format("%Y-%m-%d %H:%M").to_string();

        // Format duration
        let duration_str = entry
            .log
            .metadata
            .as_ref()
            .and_then(|m| m.duration_ms)
            .map(format_duration)
            .unwrap_or_else(|| "-".to_string());

        // Format exit code - highlight non-zero if terminal
        let exit_str = if entry.log.exit_code == 0 {
            "0".to_string()
        } else if is_terminal {
            format!("\x1b[31m{}\x1b[0m", entry.log.exit_code) // Red for non-zero
        } else {
            entry.log.exit_code.to_string()
        };

        if has_cost_data {
            // Format cost
            let cost_str = entry
                .log
                .metadata
                .as_ref()
                .and_then(|m| m.cost_usd)
                .map(|c| format!("${:.4}", c))
                .unwrap_or_else(|| "-".to_string());

            println!(
                "{:<20} {:<4} {:<35} {:<17} {:<10} {:<5} {:<10}",
                entry.session_slug,
                entry.log.sequence,
                project_display,
                date_str,
                duration_str,
                exit_str,
                cost_str
            );
        } else {
            println!(
                "{:<20} {:<4} {:<40} {:<17} {:<10} {:<5}",
                entry.session_slug,
                entry.log.sequence,
                project_display,
                date_str,
                duration_str,
                exit_str
            );
        }
    }

    // Print summary
    println!();
    if has_cost_data {
        println!("{}", "-".repeat(105));
    } else {
        println!("{}", "-".repeat(98));
    }

    let mut summary_parts = vec![format!("{} iteration(s)", iterations.len())];

    if let Some(cost) = total_cost {
        summary_parts.push(format!("Total cost: ${:.4}", cost));
    }

    if let Some(duration) = total_duration {
        summary_parts.push(format!("Total duration: {}", format_duration(duration)));
    }

    println!("{}", summary_parts.join(" | "));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iteration::{write_iteration_log, Chunk};
    use chrono::Utc;
    use tempfile::TempDir;

    fn create_test_iteration(sequence: u32, exit_code: i32) -> IterationLog {
        IterationLog {
            sequence,
            started_at: Utc::now(),
            completed_at: Utc::now(),
            exit_code,
            pending_before: 5,
            pending_after: 4,
            prompt: None,
            response: None,
            metadata: None,
            tool_calls: vec![],
            chunks: vec![Chunk::prose(format!("Iteration {}", sequence))],
            output_blocks: vec![],
        }
    }

    #[test]
    fn test_format_duration_ms() {
        assert_eq!(format_duration(500), "500ms");
        assert_eq!(format_duration(999), "999ms");
    }

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_duration(1000), "1.0s");
        assert_eq!(format_duration(45200), "45.2s");
        assert_eq!(format_duration(59999), "60.0s");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(60_000), "1m 0s");
        assert_eq!(format_duration(90_000), "1m 30s");
        assert_eq!(format_duration(125_000), "2m 5s");
    }

    #[test]
    fn test_load_iteration_log() {
        let temp_dir = TempDir::new().unwrap();
        let session_dir = temp_dir.path().to_path_buf();

        let log = create_test_iteration(1, 0);
        write_iteration_log(&session_dir, &log).unwrap();

        let loaded = load_iteration_log(&session_dir.join("iteration-1.toml")).unwrap();
        assert_eq!(loaded.sequence, 1);
        assert_eq!(loaded.exit_code, 0);
    }

    #[test]
    fn test_iterations_filter_default() {
        let filter = IterationsFilter::default();
        assert!(filter.session.is_none());
        assert!(filter.project.is_none());
        assert!(filter.outcome.is_none());
    }

    #[test]
    fn test_invalid_outcome_filter() {
        let filter = IterationsFilter {
            outcome: Some("invalid".to_string()),
            ..Default::default()
        };
        let result = list_iterations(filter);
        assert!(matches!(
            result,
            Err(IterationsError::InvalidOutcomeFilter { .. })
        ));
    }

    #[test]
    fn test_valid_outcome_filter_completed() {
        // Just verify it doesn't return InvalidOutcomeFilter error
        let filter = IterationsFilter {
            outcome: Some("completed".to_string()),
            ..Default::default()
        };
        // This may fail for other reasons (no sessions), but not for invalid filter
        let result = list_iterations(filter);
        assert!(!matches!(
            result,
            Err(IterationsError::InvalidOutcomeFilter { .. })
        ));
    }

    #[test]
    fn test_valid_outcome_filter_failed() {
        let filter = IterationsFilter {
            outcome: Some("failed".to_string()),
            ..Default::default()
        };
        let result = list_iterations(filter);
        assert!(!matches!(
            result,
            Err(IterationsError::InvalidOutcomeFilter { .. })
        ));
    }

    #[test]
    fn test_outcome_filter_case_insensitive() {
        // COMPLETED should work
        let filter = IterationsFilter {
            outcome: Some("COMPLETED".to_string()),
            ..Default::default()
        };
        let result = list_iterations(filter);
        assert!(!matches!(
            result,
            Err(IterationsError::InvalidOutcomeFilter { .. })
        ));

        // Failed should work
        let filter = IterationsFilter {
            outcome: Some("Failed".to_string()),
            ..Default::default()
        };
        let result = list_iterations(filter);
        assert!(!matches!(
            result,
            Err(IterationsError::InvalidOutcomeFilter { .. })
        ));
    }

    #[test]
    fn test_error_display_invalid_outcome() {
        let err = IterationsError::InvalidOutcomeFilter {
            value: "bad".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("bad"));
        assert!(msg.contains("completed"));
        assert!(msg.contains("failed"));
    }

    #[test]
    fn test_iterations_result_fields() {
        let result = IterationsResult {
            count: 5,
            total_cost_usd: Some(0.5),
            total_duration_ms: Some(60_000),
        };
        assert_eq!(result.count, 5);
        assert_eq!(result.total_cost_usd, Some(0.5));
        assert_eq!(result.total_duration_ms, Some(60_000));
    }
}
