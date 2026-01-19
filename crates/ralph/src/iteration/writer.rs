//! File I/O operations for iteration logs (Imperative Shell).

use crate::iteration::{IterationError, IterationLog};
use std::fs;
use std::path::Path;

/// Write an iteration log to disk.
///
/// Creates a file named iteration-N.toml in the session directory where N is
/// the sequence number.
///
/// # Arguments
///
/// * `session_dir` - Path to the session directory (e.g., ~/.config/ralph/sessions/quiet-mountain/)
/// * `log` - The iteration log to write
///
/// # Returns
///
/// * `Ok(PathBuf)` - Path to the written log file
/// * `Err(IterationError)` - If writing fails
///
/// # Example
///
/// ```no_run
/// use ralph::iteration::{IterationLog, Chunk, LogMetadata, LogToolCall, write_iteration_log};
/// use ralph_core::stream::Usage;
/// use std::path::PathBuf;
/// use chrono::Utc;
/// use serde_json::json;
///
/// let session_dir = PathBuf::from("/home/user/.config/ralph/sessions/test-session");
/// let log = IterationLog {
///     sequence: 1,
///     started_at: Utc::now(),
///     completed_at: Utc::now(),
///     exit_code: 0,
///     pending_before: 5,
///     pending_after: 4,
///     metadata: Some(LogMetadata {
///         claude_session_id: Some("abc-123".to_string()),
///         model: Some("claude-opus-4-5".to_string()),
///         cost_usd: Some(0.05),
///         duration_ms: Some(10000),
///         usage: Some(Usage::default()),
///     }),
///     tool_calls: vec![
///         LogToolCall {
///             id: "toolu_01".to_string(),
///             name: "Read".to_string(),
///             input: json!({"file_path": "/src/main.rs"}),
///             result: Some("fn main() {}".to_string()),
///             result_truncated: false,
///             is_error: false,
///         },
///     ],
///     chunks: vec![Chunk::prose("Implementation complete".to_string())],
///     output_blocks: vec![],
/// };
///
/// write_iteration_log(&session_dir, &log).unwrap();
/// ```
pub fn write_iteration_log(
    session_dir: &Path,
    log: &IterationLog,
) -> Result<std::path::PathBuf, IterationError> {
    let log_path = session_dir.join(format!("iteration-{}.toml", log.sequence));

    let content = toml::to_string_pretty(log)?;

    fs::write(&log_path, content).map_err(|e| IterationError::WriteLog {
        path: log_path.display().to_string(),
        source: e,
    })?;

    Ok(log_path)
}
