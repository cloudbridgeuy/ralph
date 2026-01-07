//! Iteration log writing (Imperative Shell).
//!
//! This module handles writing iteration logs to disk after each LLM invocation.
//! Each iteration is stored as iteration-N.toml in the session directory with:
//! - Sequence number and timestamps
//! - Exit code from the subprocess
//! - Pending story counts before and after the iteration
//! - Metadata from JSON streaming output (session_id, model, cost, usage)
//! - Output chunks (prose, code, diff blocks)
//!
//! The metadata section is populated from Claude's `--output-format stream-json`
//! events, extracting session information from system init events and cost/usage
//! data from result events.

#![allow(dead_code)] // Module not yet used by CLI commands

use chrono::{DateTime, Utc};
use ralph_core::stream::{IterationCosts, IterationMetadata, ToolInteraction, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Maximum size in bytes for tool results before truncation.
/// Results larger than this will be truncated with an indicator.
const MAX_RESULT_SIZE: usize = 10_000;

/// Metadata extracted from Claude's JSON streaming output.
///
/// This struct combines session information from system init events with
/// cost and usage data from result events. It matches the `[metadata]` section
/// in iteration-N.toml files.
///
/// # Example TOML output
///
/// ```toml
/// [metadata]
/// claude_session_id = "f5b6aaac-4316-454a-b086-a3f9e4351b1e"
/// model = "claude-opus-4-5-20251101"
/// cost_usd = 0.226354
/// duration_ms = 40966
///
/// [metadata.usage]
/// input_tokens = 712
/// output_tokens = 2971
/// cache_read_input_tokens = 107476
/// cache_creation_input_tokens = 12504
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LogMetadata {
    /// Unique identifier for this Claude session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,

    /// The Claude model being used (e.g., "claude-opus-4-5-20251101").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Total cost in USD for this iteration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,

    /// Duration of the iteration in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Token usage statistics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl LogMetadata {
    /// Create a new empty metadata struct.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create metadata from extracted `IterationMetadata` and `IterationCosts`.
    ///
    /// This combines the session information (from system init events) with
    /// cost/usage data (from result events) into a single struct suitable
    /// for serialization to TOML.
    ///
    /// # Arguments
    ///
    /// * `metadata` - Session metadata extracted from system init event
    /// * `costs` - Cost and usage data extracted from result event
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::iteration::LogMetadata;
    /// use ralph_core::stream::{IterationMetadata, IterationCosts, Usage};
    ///
    /// let metadata = IterationMetadata {
    ///     session_id: Some("abc-123".to_string()),
    ///     model: Some("claude-opus-4-5".to_string()),
    ///     tools: vec![],
    /// };
    ///
    /// let costs = IterationCosts {
    ///     cost_usd: Some(0.05),
    ///     duration_ms: Some(10000),
    ///     usage: Some(Usage::default()),
    /// };
    ///
    /// let log_metadata = LogMetadata::from_extracted(metadata, costs);
    /// assert_eq!(log_metadata.claude_session_id, Some("abc-123".to_string()));
    /// assert_eq!(log_metadata.cost_usd, Some(0.05));
    /// ```
    pub fn from_extracted(metadata: IterationMetadata, costs: IterationCosts) -> Self {
        Self {
            claude_session_id: metadata.session_id,
            model: metadata.model,
            cost_usd: costs.cost_usd,
            duration_ms: costs.duration_ms,
            usage: costs.usage,
        }
    }

    /// Check if all metadata fields are empty/None.
    pub fn is_empty(&self) -> bool {
        self.claude_session_id.is_none()
            && self.model.is_none()
            && self.cost_usd.is_none()
            && self.duration_ms.is_none()
            && self.usage.is_none()
    }
}

/// A tool call recorded in an iteration log.
///
/// This struct stores information about a single tool invocation and its result.
/// Large results are truncated to keep log files manageable.
///
/// # Example TOML output
///
/// ```toml
/// [[tool_calls]]
/// id = "toolu_01YWLzHW2VBHQSz8VV1oCGSp"
/// name = "Glob"
/// input = { pattern = ".github/workflows/*.yml" }
/// result = "/Users/.../release.yml\n/Users/.../ci.yml"
/// result_truncated = false
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogToolCall {
    /// Unique identifier for this tool use.
    pub id: String,

    /// The name of the tool that was invoked (e.g., "Read", "Edit", "Glob").
    pub name: String,

    /// The input arguments to the tool as a JSON object.
    pub input: Value,

    /// The result content from the tool execution, if available.
    /// May be truncated for large results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,

    /// Whether the result was truncated due to size.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub result_truncated: bool,

    /// Whether the tool execution resulted in an error.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

impl LogToolCall {
    /// Create a LogToolCall from a ToolInteraction.
    ///
    /// Large results are automatically truncated to `MAX_RESULT_SIZE` bytes
    /// with a truncation indicator appended.
    ///
    /// # Arguments
    ///
    /// * `interaction` - The tool interaction to convert
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::iteration::LogToolCall;
    /// use ralph_core::stream::ToolInteraction;
    /// use serde_json::json;
    ///
    /// let interaction = ToolInteraction {
    ///     id: "toolu_01".to_string(),
    ///     name: "Read".to_string(),
    ///     input: json!({"file_path": "/src/main.rs"}),
    ///     result: Some("fn main() {}".to_string()),
    ///     is_error: false,
    /// };
    ///
    /// let log_call = LogToolCall::from_interaction(&interaction);
    /// assert_eq!(log_call.name, "Read");
    /// assert!(!log_call.result_truncated);
    /// ```
    pub fn from_interaction(interaction: &ToolInteraction) -> Self {
        let (result, result_truncated) = match &interaction.result {
            Some(content) => truncate_result(content),
            None => (None, false),
        };

        Self {
            id: interaction.id.clone(),
            name: interaction.name.clone(),
            input: interaction.input.clone(),
            result,
            result_truncated,
            is_error: interaction.is_error,
        }
    }

    /// Create LogToolCalls from a slice of ToolInteractions.
    ///
    /// This is a convenience method that converts all interactions
    /// and preserves their order.
    pub fn from_interactions(interactions: &[ToolInteraction]) -> Vec<Self> {
        interactions
            .iter()
            .map(LogToolCall::from_interaction)
            .collect()
    }
}

/// Truncate a result string if it exceeds MAX_RESULT_SIZE.
///
/// Returns the (possibly truncated) result and a flag indicating whether
/// truncation occurred.
fn truncate_result(content: &str) -> (Option<String>, bool) {
    if content.len() <= MAX_RESULT_SIZE {
        (Some(content.to_string()), false)
    } else {
        // Find a safe truncation point (don't cut in the middle of a UTF-8 char)
        // We need to find the last valid char boundary at or before MAX_RESULT_SIZE
        let safe_end = content
            .char_indices()
            .take_while(|(i, _)| *i < MAX_RESULT_SIZE)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        let truncated = &content[..safe_end];
        let result = format!(
            "{}\n\n... [truncated, {} bytes total]",
            truncated,
            content.len()
        );
        (Some(result), true)
    }
}

/// A single iteration log entry.
///
/// This struct represents the complete log for one LLM invocation, stored as
/// iteration-N.toml in the session directory.
///
/// # Example TOML output
///
/// ```toml
/// sequence = 1
/// started_at = "2025-01-06T14:30:00Z"
/// completed_at = "2025-01-06T14:35:00Z"
/// exit_code = 0
/// pending_before = 5
/// pending_after = 4
///
/// [metadata]
/// claude_session_id = "f5b6aaac-4316-454a-b086-a3f9e4351b1e"
/// model = "claude-opus-4-5-20251101"
/// cost_usd = 0.226354
/// duration_ms = 40966
///
/// [metadata.usage]
/// input_tokens = 712
/// output_tokens = 2971
///
/// [[tool_calls]]
/// id = "toolu_01YWLzHW2VBHQSz8VV1oCGSp"
/// name = "Glob"
/// input = { pattern = ".github/workflows/*.yml" }
/// result = "/Users/.../release.yml\n/Users/.../ci.yml"
///
/// [[tool_calls]]
/// id = "toolu_01KKvyfhUNr2Bdu32AKbDzmX"
/// name = "Read"
/// input = { file_path = "/Users/.../Cargo.toml" }
/// result = "[workspace]\nmembers = ..."
/// result_truncated = true
///
/// [[chunks]]
/// type = "prose"
/// content = "I'll implement the feature..."
/// ```
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
    /// Metadata extracted from Claude's JSON streaming output.
    /// Contains session_id, model, cost, duration, and token usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LogMetadata>,
    /// Tool calls made during this iteration with their results.
    /// Large results are truncated to keep log files manageable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<LogToolCall>,
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
    use ralph_core::stream::{IterationCosts, IterationMetadata};
    use tempfile::TempDir;

    // ==========================================================================
    // Chunk Tests
    // ==========================================================================

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

    // ==========================================================================
    // LogMetadata Tests
    // ==========================================================================

    #[test]
    fn test_log_metadata_new() {
        let metadata = LogMetadata::new();
        assert!(metadata.is_empty());
        assert!(metadata.claude_session_id.is_none());
        assert!(metadata.model.is_none());
        assert!(metadata.cost_usd.is_none());
        assert!(metadata.duration_ms.is_none());
        assert!(metadata.usage.is_none());
    }

    #[test]
    fn test_log_metadata_is_empty() {
        let empty = LogMetadata::default();
        assert!(empty.is_empty());

        let with_session_id = LogMetadata {
            claude_session_id: Some("abc".to_string()),
            ..Default::default()
        };
        assert!(!with_session_id.is_empty());

        let with_cost = LogMetadata {
            cost_usd: Some(0.05),
            ..Default::default()
        };
        assert!(!with_cost.is_empty());
    }

    #[test]
    fn test_log_metadata_from_extracted() {
        let metadata = IterationMetadata {
            session_id: Some("abc-123".to_string()),
            model: Some("claude-opus-4-5".to_string()),
            tools: vec![],
        };

        let costs = IterationCosts {
            cost_usd: Some(0.05),
            duration_ms: Some(10000),
            usage: Some(Usage {
                input_tokens: 100,
                output_tokens: 200,
                cache_read_input_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
            }),
        };

        let log_metadata = LogMetadata::from_extracted(metadata, costs);

        assert_eq!(log_metadata.claude_session_id, Some("abc-123".to_string()));
        assert_eq!(log_metadata.model, Some("claude-opus-4-5".to_string()));
        assert_eq!(log_metadata.cost_usd, Some(0.05));
        assert_eq!(log_metadata.duration_ms, Some(10000));
        assert!(log_metadata.usage.is_some());
        assert_eq!(log_metadata.usage.as_ref().unwrap().input_tokens, 100);
    }

    #[test]
    fn test_log_metadata_serialization() {
        let metadata = LogMetadata {
            claude_session_id: Some("f5b6aaac-4316-454a-b086-a3f9e4351b1e".to_string()),
            model: Some("claude-opus-4-5-20251101".to_string()),
            cost_usd: Some(0.226354),
            duration_ms: Some(40966),
            usage: Some(Usage {
                input_tokens: 712,
                output_tokens: 2971,
                cache_read_input_tokens: Some(107476),
                cache_creation_input_tokens: Some(12504),
            }),
        };

        let toml_str = toml::to_string_pretty(&metadata).unwrap();

        assert!(toml_str.contains("claude_session_id = \"f5b6aaac-4316-454a-b086-a3f9e4351b1e\""));
        assert!(toml_str.contains("model = \"claude-opus-4-5-20251101\""));
        assert!(toml_str.contains("cost_usd = 0.226354"));
        assert!(toml_str.contains("duration_ms = 40966"));
        assert!(toml_str.contains("[usage]"));
        assert!(toml_str.contains("input_tokens = 712"));
        assert!(toml_str.contains("output_tokens = 2971"));
    }

    #[test]
    fn test_log_metadata_deserialization() {
        let toml_str = r#"
            claude_session_id = "abc-123"
            model = "claude-opus-4-5"
            cost_usd = 0.05
            duration_ms = 10000

            [usage]
            input_tokens = 100
            output_tokens = 200
            cache_read_input_tokens = 0
            cache_creation_input_tokens = 0
        "#;

        let metadata: LogMetadata = toml::from_str(toml_str).unwrap();

        assert_eq!(metadata.claude_session_id, Some("abc-123".to_string()));
        assert_eq!(metadata.model, Some("claude-opus-4-5".to_string()));
        assert_eq!(metadata.cost_usd, Some(0.05));
        assert_eq!(metadata.duration_ms, Some(10000));
        assert!(metadata.usage.is_some());
    }

    #[test]
    fn test_log_metadata_empty_fields_skipped() {
        let metadata = LogMetadata {
            claude_session_id: Some("abc".to_string()),
            model: None,
            cost_usd: None,
            duration_ms: None,
            usage: None,
        };

        let toml_str = toml::to_string(&metadata).unwrap();

        // Only claude_session_id should appear
        assert!(toml_str.contains("claude_session_id"));
        assert!(!toml_str.contains("model"));
        assert!(!toml_str.contains("cost_usd"));
        assert!(!toml_str.contains("duration_ms"));
        assert!(!toml_str.contains("usage"));
    }

    // ==========================================================================
    // IterationLog Tests (without metadata)
    // ==========================================================================

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
            metadata: None,
            tool_calls: vec![],
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
        // Metadata should not appear when None
        assert!(!toml_str.contains("[metadata]"));
        // tool_calls should not appear when empty
        assert!(!toml_str.contains("[[tool_calls]]"));
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
        assert!(log.metadata.is_none()); // Backward compatible - no metadata
        assert!(log.tool_calls.is_empty()); // Backward compatible - no tool_calls
        assert_eq!(log.chunks.len(), 1);
        assert_eq!(log.chunks[0].chunk_type, "prose");
        assert_eq!(log.chunks[0].content, "Implementation complete");
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
            metadata: None,
            tool_calls: vec![],
            chunks: vec![],
        };

        let toml_str = toml::to_string_pretty(&log).unwrap();
        let parsed: IterationLog = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.chunks.len(), 0);
        assert!(parsed.tool_calls.is_empty());
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
            metadata: None,
            tool_calls: vec![],
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

    // ==========================================================================
    // IterationLog Tests (with metadata)
    // ==========================================================================

    #[test]
    fn test_iteration_log_with_metadata() {
        let now = Utc::now();
        let log = IterationLog {
            sequence: 1,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 5,
            pending_after: 4,
            metadata: Some(LogMetadata {
                claude_session_id: Some("f5b6aaac-4316-454a-b086-a3f9e4351b1e".to_string()),
                model: Some("claude-opus-4-5-20251101".to_string()),
                cost_usd: Some(0.226354),
                duration_ms: Some(40966),
                usage: Some(Usage {
                    input_tokens: 712,
                    output_tokens: 2971,
                    cache_read_input_tokens: Some(107476),
                    cache_creation_input_tokens: Some(12504),
                }),
            }),
            tool_calls: vec![],
            chunks: vec![Chunk::prose("Test output".to_string())],
        };

        let toml_str = toml::to_string_pretty(&log).unwrap();

        // Verify metadata section is present
        assert!(toml_str.contains("[metadata]"));
        assert!(toml_str.contains("claude_session_id = \"f5b6aaac-4316-454a-b086-a3f9e4351b1e\""));
        assert!(toml_str.contains("model = \"claude-opus-4-5-20251101\""));
        assert!(toml_str.contains("cost_usd = 0.226354"));
        assert!(toml_str.contains("duration_ms = 40966"));
        assert!(toml_str.contains("[metadata.usage]"));
        assert!(toml_str.contains("input_tokens = 712"));
    }

    #[test]
    fn test_iteration_log_deserialization_with_metadata() {
        let toml_str = r#"
            sequence = 1
            started_at = "2025-01-06T14:30:00Z"
            completed_at = "2025-01-06T14:35:00Z"
            exit_code = 0
            pending_before = 5
            pending_after = 4

            [metadata]
            claude_session_id = "abc-123"
            model = "claude-opus-4-5"
            cost_usd = 0.05
            duration_ms = 10000

            [metadata.usage]
            input_tokens = 100
            output_tokens = 200
            cache_read_input_tokens = 50
            cache_creation_input_tokens = 25

            [[chunks]]
            type = "prose"
            content = "Implementation complete"
        "#;

        let log: IterationLog = toml::from_str(toml_str).unwrap();

        assert_eq!(log.sequence, 1);
        assert!(log.metadata.is_some());

        let metadata = log.metadata.unwrap();
        assert_eq!(metadata.claude_session_id, Some("abc-123".to_string()));
        assert_eq!(metadata.model, Some("claude-opus-4-5".to_string()));
        assert_eq!(metadata.cost_usd, Some(0.05));
        assert_eq!(metadata.duration_ms, Some(10000));

        let usage = metadata.usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 200);
        assert_eq!(usage.cache_read_input_tokens, Some(50));
        assert_eq!(usage.cache_creation_input_tokens, Some(25));
    }

    #[test]
    fn test_iteration_log_backward_compatible_without_metadata() {
        // Old logs without metadata section should still parse
        let old_toml = r#"
            sequence = 3
            started_at = "2025-01-06T14:30:00Z"
            completed_at = "2025-01-06T14:35:00Z"
            exit_code = 0
            pending_before = 10
            pending_after = 9

            [[chunks]]
            type = "prose"
            content = "Old format log"
        "#;

        let log: IterationLog = toml::from_str(old_toml).unwrap();

        assert_eq!(log.sequence, 3);
        assert!(log.metadata.is_none());
        assert_eq!(log.chunks.len(), 1);
    }

    // ==========================================================================
    // File Writing Tests
    // ==========================================================================

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
            metadata: None,
            tool_calls: vec![],
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
    fn test_write_iteration_log_with_metadata() {
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
            metadata: Some(LogMetadata {
                claude_session_id: Some("test-session-id".to_string()),
                model: Some("claude-opus-4-5".to_string()),
                cost_usd: Some(0.123),
                duration_ms: Some(5000),
                usage: Some(Usage {
                    input_tokens: 500,
                    output_tokens: 1000,
                    cache_read_input_tokens: Some(0),
                    cache_creation_input_tokens: Some(0),
                }),
            }),
            tool_calls: vec![],
            chunks: vec![Chunk::prose("Test output".to_string())],
        };

        let log_path = write_iteration_log(session_dir, &log).unwrap();

        // Verify content can be read back with metadata
        let content = fs::read_to_string(&log_path).unwrap();
        let parsed: IterationLog = toml::from_str(&content).unwrap();

        assert!(parsed.metadata.is_some());
        let metadata = parsed.metadata.unwrap();
        assert_eq!(
            metadata.claude_session_id,
            Some("test-session-id".to_string())
        );
        assert_eq!(metadata.cost_usd, Some(0.123));
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
            metadata: None,
            tool_calls: vec![],
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
            metadata: None,
            tool_calls: vec![],
            chunks: vec![Chunk::prose("Second iteration".to_string())],
        };
        write_iteration_log(session_dir, &log2).unwrap();

        // Verify both files exist
        assert!(session_dir.join("iteration-1.toml").exists());
        assert!(session_dir.join("iteration-2.toml").exists());
    }

    // ==========================================================================
    // LogToolCall Tests
    // ==========================================================================

    #[test]
    fn test_log_tool_call_from_interaction() {
        use ralph_core::stream::ToolInteraction;
        use serde_json::json;

        let interaction = ToolInteraction {
            id: "toolu_01".to_string(),
            name: "Read".to_string(),
            input: json!({"file_path": "/src/main.rs"}),
            result: Some("fn main() {}".to_string()),
            is_error: false,
        };

        let log_call = LogToolCall::from_interaction(&interaction);

        assert_eq!(log_call.id, "toolu_01");
        assert_eq!(log_call.name, "Read");
        assert_eq!(log_call.input, json!({"file_path": "/src/main.rs"}));
        assert_eq!(log_call.result, Some("fn main() {}".to_string()));
        assert!(!log_call.result_truncated);
        assert!(!log_call.is_error);
    }

    #[test]
    fn test_log_tool_call_from_interaction_no_result() {
        use ralph_core::stream::ToolInteraction;
        use serde_json::json;

        let interaction = ToolInteraction {
            id: "toolu_01".to_string(),
            name: "Edit".to_string(),
            input: json!({"file_path": "/src/main.rs", "old_string": "a", "new_string": "b"}),
            result: None,
            is_error: false,
        };

        let log_call = LogToolCall::from_interaction(&interaction);

        assert_eq!(log_call.name, "Edit");
        assert!(log_call.result.is_none());
        assert!(!log_call.result_truncated);
    }

    #[test]
    fn test_log_tool_call_from_interaction_error() {
        use ralph_core::stream::ToolInteraction;
        use serde_json::json;

        let interaction = ToolInteraction {
            id: "toolu_01".to_string(),
            name: "Read".to_string(),
            input: json!({"file_path": "/nonexistent"}),
            result: Some("File not found".to_string()),
            is_error: true,
        };

        let log_call = LogToolCall::from_interaction(&interaction);

        assert_eq!(log_call.result, Some("File not found".to_string()));
        assert!(log_call.is_error);
    }

    #[test]
    fn test_log_tool_call_truncation() {
        use ralph_core::stream::ToolInteraction;
        use serde_json::json;

        // Create a large result that exceeds MAX_RESULT_SIZE
        let large_content = "x".repeat(15_000);

        let interaction = ToolInteraction {
            id: "toolu_01".to_string(),
            name: "Read".to_string(),
            input: json!({"file_path": "/src/large_file.rs"}),
            result: Some(large_content.clone()),
            is_error: false,
        };

        let log_call = LogToolCall::from_interaction(&interaction);

        assert!(log_call.result_truncated);
        assert!(log_call.result.is_some());

        let result = log_call.result.unwrap();
        assert!(result.contains("... [truncated, 15000 bytes total]"));
        // Result should be smaller than original
        assert!(result.len() < large_content.len());
    }

    #[test]
    fn test_log_tool_call_no_truncation_for_small_result() {
        use ralph_core::stream::ToolInteraction;
        use serde_json::json;

        let content = "x".repeat(5_000);

        let interaction = ToolInteraction {
            id: "toolu_01".to_string(),
            name: "Read".to_string(),
            input: json!({"file_path": "/src/file.rs"}),
            result: Some(content.clone()),
            is_error: false,
        };

        let log_call = LogToolCall::from_interaction(&interaction);

        assert!(!log_call.result_truncated);
        assert_eq!(log_call.result, Some(content));
    }

    #[test]
    fn test_log_tool_call_from_interactions() {
        use ralph_core::stream::ToolInteraction;
        use serde_json::json;

        let interactions = vec![
            ToolInteraction {
                id: "toolu_01".to_string(),
                name: "Glob".to_string(),
                input: json!({"pattern": "*.rs"}),
                result: Some("src/main.rs\nsrc/lib.rs".to_string()),
                is_error: false,
            },
            ToolInteraction {
                id: "toolu_02".to_string(),
                name: "Read".to_string(),
                input: json!({"file_path": "/src/main.rs"}),
                result: Some("fn main() {}".to_string()),
                is_error: false,
            },
        ];

        let log_calls = LogToolCall::from_interactions(&interactions);

        assert_eq!(log_calls.len(), 2);
        assert_eq!(log_calls[0].name, "Glob");
        assert_eq!(log_calls[1].name, "Read");
    }

    #[test]
    fn test_log_tool_call_serialization() {
        use serde_json::json;

        let log_call = LogToolCall {
            id: "toolu_01".to_string(),
            name: "Glob".to_string(),
            input: json!({"pattern": "*.rs"}),
            result: Some("src/main.rs".to_string()),
            result_truncated: false,
            is_error: false,
        };

        let toml_str = toml::to_string(&log_call).unwrap();

        assert!(toml_str.contains("id = \"toolu_01\""));
        assert!(toml_str.contains("name = \"Glob\""));
        assert!(toml_str.contains("result = \"src/main.rs\""));
        // Boolean false fields should not appear when using skip_serializing_if
        assert!(!toml_str.contains("result_truncated"));
        assert!(!toml_str.contains("is_error"));
    }

    #[test]
    fn test_log_tool_call_serialization_with_truncated() {
        use serde_json::json;

        let log_call = LogToolCall {
            id: "toolu_01".to_string(),
            name: "Read".to_string(),
            input: json!({"file_path": "/src/main.rs"}),
            result: Some("truncated content...".to_string()),
            result_truncated: true,
            is_error: false,
        };

        let toml_str = toml::to_string(&log_call).unwrap();

        assert!(toml_str.contains("result_truncated = true"));
    }

    #[test]
    fn test_log_tool_call_serialization_with_error() {
        use serde_json::json;

        let log_call = LogToolCall {
            id: "toolu_01".to_string(),
            name: "Read".to_string(),
            input: json!({"file_path": "/nonexistent"}),
            result: Some("File not found".to_string()),
            result_truncated: false,
            is_error: true,
        };

        let toml_str = toml::to_string(&log_call).unwrap();

        assert!(toml_str.contains("is_error = true"));
    }

    #[test]
    fn test_log_tool_call_deserialization() {
        let toml_str = r#"
            id = "toolu_01YWLzHW2VBHQSz8VV1oCGSp"
            name = "Glob"
            result = "/Users/dev/project/release.yml\n/Users/dev/project/ci.yml"

            [input]
            pattern = ".github/workflows/*.yml"
        "#;

        let log_call: LogToolCall = toml::from_str(toml_str).unwrap();

        assert_eq!(log_call.id, "toolu_01YWLzHW2VBHQSz8VV1oCGSp");
        assert_eq!(log_call.name, "Glob");
        assert!(log_call.result.is_some());
        assert!(!log_call.result_truncated);
        assert!(!log_call.is_error);
    }

    #[test]
    fn test_iteration_log_with_tool_calls() {
        use serde_json::json;

        let now = Utc::now();
        let log = IterationLog {
            sequence: 1,
            started_at: now,
            completed_at: now,
            exit_code: 0,
            pending_before: 5,
            pending_after: 4,
            metadata: None,
            tool_calls: vec![
                LogToolCall {
                    id: "toolu_01".to_string(),
                    name: "Glob".to_string(),
                    input: json!({"pattern": "*.rs"}),
                    result: Some("src/main.rs".to_string()),
                    result_truncated: false,
                    is_error: false,
                },
                LogToolCall {
                    id: "toolu_02".to_string(),
                    name: "Read".to_string(),
                    input: json!({"file_path": "/src/main.rs"}),
                    result: Some("fn main() {}".to_string()),
                    result_truncated: false,
                    is_error: false,
                },
            ],
            chunks: vec![Chunk::prose("Found and read files".to_string())],
        };

        let toml_str = toml::to_string_pretty(&log).unwrap();

        assert!(toml_str.contains("[[tool_calls]]"));
        assert!(toml_str.contains("name = \"Glob\""));
        assert!(toml_str.contains("name = \"Read\""));

        // Verify round-trip
        let parsed: IterationLog = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.tool_calls.len(), 2);
        assert_eq!(parsed.tool_calls[0].name, "Glob");
        assert_eq!(parsed.tool_calls[1].name, "Read");
    }

    #[test]
    fn test_iteration_log_deserialization_with_tool_calls() {
        let toml_str = r#"
            sequence = 1
            started_at = "2025-01-06T14:30:00Z"
            completed_at = "2025-01-06T14:35:00Z"
            exit_code = 0
            pending_before = 5
            pending_after = 4

            [[tool_calls]]
            id = "toolu_01YWLzHW2VBHQSz8VV1oCGSp"
            name = "Glob"
            result = "/Users/.../release.yml\n/Users/.../ci.yml"

            [tool_calls.input]
            pattern = ".github/workflows/*.yml"

            [[tool_calls]]
            id = "toolu_01KKvyfhUNr2Bdu32AKbDzmX"
            name = "Read"
            result = "[workspace]\nmembers = ..."
            result_truncated = true

            [tool_calls.input]
            file_path = "/Users/.../Cargo.toml"

            [[chunks]]
            type = "prose"
            content = "I'll check the workflows..."
        "#;

        let log: IterationLog = toml::from_str(toml_str).unwrap();

        assert_eq!(log.tool_calls.len(), 2);

        let first_call = &log.tool_calls[0];
        assert_eq!(first_call.name, "Glob");
        assert!(!first_call.result_truncated);

        let second_call = &log.tool_calls[1];
        assert_eq!(second_call.name, "Read");
        assert!(second_call.result_truncated);
    }

    #[test]
    fn test_write_iteration_log_with_tool_calls() {
        use serde_json::json;

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
            metadata: None,
            tool_calls: vec![LogToolCall {
                id: "toolu_01".to_string(),
                name: "Glob".to_string(),
                input: json!({"pattern": "*.rs"}),
                result: Some("src/main.rs".to_string()),
                result_truncated: false,
                is_error: false,
            }],
            chunks: vec![Chunk::prose("Found files".to_string())],
        };

        let log_path = write_iteration_log(session_dir, &log).unwrap();

        let content = fs::read_to_string(&log_path).unwrap();
        let parsed: IterationLog = toml::from_str(&content).unwrap();

        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "Glob");
    }

    #[test]
    fn test_truncate_result_preserves_utf8() {
        // Test that truncation doesn't break UTF-8 characters
        // Create a string with multi-byte UTF-8 characters
        let content = "こんにちは世界".repeat(2000); // Japanese text

        let (result, truncated) = truncate_result(&content);

        assert!(truncated);
        assert!(result.is_some());

        // The result should be valid UTF-8
        let result_str = result.unwrap();
        assert!(result_str.is_char_boundary(result_str.len()));

        // Should contain the truncation message
        assert!(result_str.contains("... [truncated,"));
    }
}
