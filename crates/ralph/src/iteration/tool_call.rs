//! LogToolCall struct for recording tool invocations in iteration logs.

use crate::iteration::MAX_RESULT_SIZE;
use ralph_core::stream::ToolInteraction;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
pub(crate) fn truncate_result(content: &str) -> (Option<String>, bool) {
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
