//! LogMetadata struct for session metadata from Claude's JSON streaming output.

use ralph_core::stream::{IterationCosts, IterationMetadata, Usage};
use serde::{Deserialize, Serialize};

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
