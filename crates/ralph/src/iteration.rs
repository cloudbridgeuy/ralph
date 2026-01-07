//! Iteration log writing (Imperative Shell).
//!
//! This module handles writing iteration logs to disk after each LLM invocation.
//! Each iteration is stored as iteration-N.toml in the session directory with:
//! - Sequence number and timestamps
//! - Exit code from the subprocess
//! - Pending story counts before and after the iteration
//! - Output chunks (currently just raw output as prose)
//!
//! This is the basic version for Layer 1. Layer 1C will add metadata extraction,
//! and Layer 3 will add typed chunk parsing (code/diff blocks).

#![allow(dead_code)] // Module not yet used by CLI commands

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// A single iteration log entry.
///
/// This struct represents the complete log for one LLM invocation, stored as
/// iteration-N.toml in the session directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationLog {
    /// Iteration sequence number (1-indexed)
    pub sequence: u32,
    /// When the iteration started
    pub started_at: DateTime<Utc>,
    /// When the iteration completed
    pub completed_at: DateTime<Utc>,
    /// Exit code from the LLM subprocess
    pub exit_code: i32,
    /// Number of pending stories before this iteration
    pub pending_before: usize,
    /// Number of pending stories after this iteration
    pub pending_after: usize,
    /// Output chunks from the LLM
    #[serde(default)]
    pub chunks: Vec<Chunk>,
}

/// A chunk of output from the LLM.
///
/// Currently basic (prose only for Layer 1). Layer 3 will add typed chunks
/// (code blocks with language hints, diff blocks).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Type of chunk: "prose", "code", or "diff"
    #[serde(rename = "type")]
    pub chunk_type: String,
    /// The actual content
    pub content: String,
    /// Optional language hint (for code chunks)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

impl Chunk {
    /// Create a prose chunk (plain text/markdown).
    pub fn prose(content: String) -> Self {
        Self {
            chunk_type: "prose".to_string(),
            content,
            language: None,
        }
    }

    /// Create a code chunk with language hint.
    #[allow(dead_code)] // Will be used in Layer 3
    pub fn code(content: String, language: String) -> Self {
        Self {
            chunk_type: "code".to_string(),
            content,
            language: Some(language),
        }
    }

    /// Create a diff chunk.
    #[allow(dead_code)] // Will be used in Layer 3
    pub fn diff(content: String) -> Self {
        Self {
            chunk_type: "diff".to_string(),
            content,
            language: None,
        }
    }
}

/// Error type for iteration log operations.
#[derive(Debug, thiserror::Error)]
pub enum IterationError {
    /// Failed to write iteration log
    #[error("Failed to write iteration log at {path}: {source}")]
    WriteLog {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to serialize iteration log
    #[error("Failed to serialize iteration log: {0}")]
    SerializeLog(#[from] toml::ser::Error),
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
/// use ralph::iteration::{IterationLog, Chunk, write_iteration_log};
/// use std::path::PathBuf;
/// use chrono::Utc;
///
/// let session_dir = PathBuf::from("/home/user/.config/ralph/sessions/test-session");
/// let log = IterationLog {
///     sequence: 1,
///     started_at: Utc::now(),
///     completed_at: Utc::now(),
///     exit_code: 0,
///     pending_before: 5,
///     pending_after: 4,
///     chunks: vec![Chunk::prose("Implementation complete".to_string())],
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_chunk_prose() {
        let chunk = Chunk::prose("Hello, world!".to_string());
        assert_eq!(chunk.chunk_type, "prose");
        assert_eq!(chunk.content, "Hello, world!");
        assert!(chunk.language.is_none());
    }

    #[test]
    fn test_chunk_code() {
        let chunk = Chunk::code("fn main() {}".to_string(), "rust".to_string());
        assert_eq!(chunk.chunk_type, "code");
        assert_eq!(chunk.content, "fn main() {}");
        assert_eq!(chunk.language, Some("rust".to_string()));
    }

    #[test]
    fn test_chunk_diff() {
        let chunk = Chunk::diff("@@ -1,3 +1,3 @@".to_string());
        assert_eq!(chunk.chunk_type, "diff");
        assert_eq!(chunk.content, "@@ -1,3 +1,3 @@");
        assert!(chunk.language.is_none());
    }

    #[test]
    fn test_iteration_log_serialization() {
        let now = Utc::now();
        let log = IterationLog {
            sequence: 1,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 5,
            pending_after: 4,
            chunks: vec![Chunk::prose("Test output".to_string())],
        };

        let toml_str = toml::to_string_pretty(&log).unwrap();

        // Verify key fields are present
        assert!(toml_str.contains("sequence = 1"));
        assert!(toml_str.contains("exit_code = 0"));
        assert!(toml_str.contains("pending_before = 5"));
        assert!(toml_str.contains("pending_after = 4"));
        assert!(toml_str.contains("[[chunks]]"));
        assert!(toml_str.contains("type = \"prose\""));
        assert!(toml_str.contains("Test output"));
    }

    #[test]
    fn test_iteration_log_deserialization() {
        // Note: TOML requires RFC 3339 dates without the Z suffix for bare datetime,
        // or quoted strings for full RFC 3339 with timezone
        let toml_str = r#"
            sequence = 2
            started_at = "2025-01-06T14:30:00Z"
            completed_at = "2025-01-06T14:35:00Z"
            exit_code = 0
            pending_before = 3
            pending_after = 2

            [[chunks]]
            type = "prose"
            content = "Implementation complete"
        "#;

        let log: IterationLog = toml::from_str(toml_str).unwrap();

        assert_eq!(log.sequence, 2);
        assert_eq!(log.exit_code, 0);
        assert_eq!(log.pending_before, 3);
        assert_eq!(log.pending_after, 2);
        assert_eq!(log.chunks.len(), 1);
        assert_eq!(log.chunks[0].chunk_type, "prose");
        assert_eq!(log.chunks[0].content, "Implementation complete");
    }

    #[test]
    fn test_write_iteration_log() {
        let temp_dir = TempDir::new().unwrap();
        let session_dir = temp_dir.path();

        let now = Utc::now();
        let log = IterationLog {
            sequence: 1,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 5,
            pending_after: 4,
            chunks: vec![
                Chunk::prose("Starting work...".to_string()),
                Chunk::prose("Finished!".to_string()),
            ],
        };

        let log_path = write_iteration_log(session_dir, &log).unwrap();

        // Verify file was created
        assert!(log_path.exists());
        assert!(log_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("iteration-1.toml"));

        // Verify content can be read back
        let content = fs::read_to_string(&log_path).unwrap();
        let parsed: IterationLog = toml::from_str(&content).unwrap();

        assert_eq!(parsed.sequence, 1);
        assert_eq!(parsed.exit_code, 0);
        assert_eq!(parsed.chunks.len(), 2);
    }

    #[test]
    fn test_write_multiple_iteration_logs() {
        let temp_dir = TempDir::new().unwrap();
        let session_dir = temp_dir.path();

        let now = Utc::now();

        // Write iteration 1
        let log1 = IterationLog {
            sequence: 1,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 5,
            pending_after: 4,
            chunks: vec![Chunk::prose("First iteration".to_string())],
        };
        write_iteration_log(session_dir, &log1).unwrap();

        // Write iteration 2
        let log2 = IterationLog {
            sequence: 2,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 4,
            pending_after: 3,
            chunks: vec![Chunk::prose("Second iteration".to_string())],
        };
        write_iteration_log(session_dir, &log2).unwrap();

        // Verify both files exist
        assert!(session_dir.join("iteration-1.toml").exists());
        assert!(session_dir.join("iteration-2.toml").exists());
    }

    #[test]
    fn test_iteration_log_with_code_chunk() {
        let now = Utc::now();
        let log = IterationLog {
            sequence: 1,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 2,
            pending_after: 1,
            chunks: vec![
                Chunk::prose("I'll implement the function:".to_string()),
                Chunk::code(
                    "fn hello() {\n    println!(\"Hello\");\n}".to_string(),
                    "rust".to_string(),
                ),
            ],
        };

        let toml_str = toml::to_string_pretty(&log).unwrap();

        // Verify language field is included for code chunks
        assert!(toml_str.contains("language = \"rust\""));

        // Verify deserialization preserves language
        let parsed: IterationLog = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.chunks[1].language, Some("rust".to_string()));
    }

    #[test]
    fn test_iteration_log_empty_chunks() {
        let now = Utc::now();
        let log = IterationLog {
            sequence: 1,
            started_at: now,
            completed_at: now,
            exit_code: 1,
            pending_before: 5,
            pending_after: 5,
            chunks: vec![],
        };

        let toml_str = toml::to_string_pretty(&log).unwrap();
        let parsed: IterationLog = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.chunks.len(), 0);
    }

    #[test]
    fn test_chunk_serialization_omits_none_language() {
        let chunk = Chunk::prose("Plain text".to_string());

        #[derive(Serialize)]
        struct Wrapper {
            chunk: Chunk,
        }

        let wrapper = Wrapper { chunk };
        let toml_str = toml::to_string(&wrapper).unwrap();

        // Language field should not appear when None
        assert!(!toml_str.contains("language"));
    }
}
