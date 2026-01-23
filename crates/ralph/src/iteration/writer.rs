//! File I/O operations for iteration logs (Imperative Shell).

use crate::iteration::{IterationError, IterationLog};
use std::fs;
use std::path::Path;

/// Message from a conversation turn (prompt and response pair).
///
/// This struct holds the extracted prompt and response from an iteration log
/// for use in building conversation history.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationMessage {
    /// The user's prompt for this turn.
    pub prompt: String,
    /// The assistant's response for this turn (may be empty if unavailable).
    pub response: String,
}

impl ConversationMessage {
    /// Creates a new ConversationMessage.
    pub fn new(prompt: String, response: String) -> Self {
        Self { prompt, response }
    }
}

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

/// Check if a filename matches the iteration log pattern.
///
/// Iteration logs are named `iteration-N.toml` where N is the sequence number.
fn is_iteration_log_file(name: &str) -> bool {
    name.starts_with("iteration-") && name.ends_with(".toml")
}

/// Count the number of existing iteration logs in a session directory.
///
/// Reads the session directory and counts files matching the pattern
/// `iteration-*.toml`. Returns the count, which can be used to determine
/// the next sequence number (count + 1).
///
/// # Arguments
///
/// * `session_dir` - Path to the session directory
///
/// # Returns
///
/// * `Ok(u32)` - The number of existing iteration log files
/// * `Err(IterationError)` - If reading the directory fails
pub fn count_iterations(session_dir: &Path) -> Result<u32, IterationError> {
    let entries = fs::read_dir(session_dir).map_err(|e| IterationError::ReadSessionDir {
        path: session_dir.display().to_string(),
        source: e,
    })?;

    let count = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(is_iteration_log_file)
                .unwrap_or(false)
        })
        .count();

    Ok(count as u32)
}

/// Load all iteration logs for a session directory, sorted by sequence number.
///
/// Reads all `iteration-*.toml` files from the session directory and returns
/// them sorted by sequence number (ascending, oldest first). This is useful
/// for building conversation history when continuing a session.
///
/// # Arguments
///
/// * `session_dir` - Path to the session directory
///
/// # Returns
///
/// * `Ok(Vec<IterationLog>)` - All iteration logs sorted by sequence
/// * `Err(IterationError)` - If reading or parsing fails
pub fn load_session_iterations(session_dir: &Path) -> Result<Vec<IterationLog>, IterationError> {
    let entries = fs::read_dir(session_dir).map_err(|e| IterationError::ReadSessionDir {
        path: session_dir.display().to_string(),
        source: e,
    })?;

    let mut logs: Vec<IterationLog> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if is_iteration_log_file(name) {
                let content = fs::read_to_string(&path).map_err(|e| IterationError::ReadLog {
                    path: path.display().to_string(),
                    source: e,
                })?;

                let log: IterationLog =
                    toml::from_str(&content).map_err(|e| IterationError::ParseLog {
                        path: path.display().to_string(),
                        source: e,
                    })?;

                logs.push(log);
            }
        }
    }

    // Sort by sequence number (ascending, oldest first)
    logs.sort_by_key(|log| log.sequence);

    Ok(logs)
}

/// Extract conversation messages from iteration logs.
///
/// This is a pure function that transforms iteration logs into conversation
/// messages by extracting the prompt and response from each log. Logs without
/// a prompt are skipped (e.g., `run` command iterations).
///
/// # Arguments
///
/// * `logs` - Slice of iteration logs to extract messages from
///
/// # Returns
///
/// A vector of conversation messages in sequence order.
pub fn extract_conversation_messages(logs: &[IterationLog]) -> Vec<ConversationMessage> {
    logs.iter()
        .filter_map(|log| {
            log.prompt.as_ref().map(|prompt| {
                ConversationMessage::new(prompt.clone(), log.response.clone().unwrap_or_default())
            })
        })
        .collect()
}
