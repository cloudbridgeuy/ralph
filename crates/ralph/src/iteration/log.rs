//! IterationLog struct for complete iteration log entries.

use crate::iteration::{Chunk, LogMetadata, LogToolCall};
use crate::stream_processor::OutputBlock;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    /// The user's prompt text for this iteration.
    ///
    /// For `ask` command sessions, this contains the user's input prompt.
    /// For `run` command sessions, this is `None` because the run command
    /// operates on PRD stories rather than user prompts per iteration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The assistant's response text for this iteration.
    /// Extracted from output_blocks (text content only, excluding tool output).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
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
    /// Output blocks for replay serialization.
    /// Contains all visual output in display order for faithful replay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_blocks: Vec<OutputBlock>,
}

/// Extract the assistant's response text from output blocks.
///
/// This pure function iterates over output blocks and concatenates the text
/// content from `Text` blocks, which represent the assistant's prose, code,
/// and diff responses. Tool invocations and results are excluded.
///
/// Returns `None` if no text content is present.
///
/// # Example
///
/// ```
/// use ralph::iteration::extract_response_text;
/// use ralph::stream_processor::OutputBlock;
/// use ralph_core::chunk::ParsedChunk;
///
/// let blocks = vec![
///     OutputBlock::text(ParsedChunk::prose("Hello, world!")),
///     OutputBlock::text(ParsedChunk::code("fn main() {}", Some("rust".to_string()))),
/// ];
///
/// let response = extract_response_text(&blocks);
/// assert!(response.is_some());
/// assert!(response.unwrap().contains("Hello, world!"));
/// ```
pub fn extract_response_text(output_blocks: &[OutputBlock]) -> Option<String> {
    let text_parts: Vec<_> = output_blocks
        .iter()
        .filter_map(|block| match block {
            OutputBlock::Text(text_block) => Some(text_block.chunk.content.as_str()),
            _ => None,
        })
        .collect();

    (!text_parts.is_empty()).then(|| text_parts.join("\n\n"))
}
