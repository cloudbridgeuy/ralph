//! Core event types for stream-JSON parsing.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A stream event from Claude CLI's `--output-format stream-json` output.
///
/// Each line of the stream output is a JSON object that deserializes to one of these variants.
/// The `type` field determines which variant is used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// System event containing initialization metadata.
    System(SystemEvent),

    /// Assistant event containing LLM response with text and/or tool calls.
    Assistant(AssistantEvent),

    /// User event containing tool execution results.
    User(UserEvent),

    /// Result event containing final costs, usage, and duration.
    Result(ResultEvent),
}

/// System event for session initialization and other system messages.
///
/// The `subtype` field indicates the kind of system event:
/// - `"init"` - Session initialization with session_id and model
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemEvent {
    /// The subtype of system event (e.g., "init").
    #[serde(default)]
    pub subtype: Option<String>,

    /// Unique identifier for this Claude session.
    #[serde(default)]
    pub session_id: Option<String>,

    /// The Claude model being used (e.g., "claude-opus-4-5-20251101").
    #[serde(default)]
    pub model: Option<String>,
}

/// Assistant event containing LLM response content.
///
/// Each assistant event represents a message from Claude. The `message` field
/// contains the actual content, which can include text and/or tool use requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantEvent {
    /// The message content from the assistant.
    pub message: AssistantMessage,
}

/// The message payload from an assistant event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    /// Unique identifier for this message.
    #[serde(default)]
    pub id: Option<String>,

    /// The content blocks in this message (text, tool_use, etc.).
    #[serde(default)]
    pub content: Vec<ContentBlock>,

    /// The model that generated this response.
    #[serde(default)]
    pub model: Option<String>,

    /// Stop reason for the message (e.g., "end_turn", "tool_use").
    #[serde(default)]
    pub stop_reason: Option<String>,

    #[serde(default)]
    pub usage: Option<Usage>,
}

/// A content block within an assistant message.
///
/// Content can be either text output from Claude or a tool use request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Text content from Claude.
    Text {
        /// The text content.
        text: String,
    },

    /// A tool use request from Claude.
    ToolUse {
        /// Unique identifier for this tool use (used to correlate with results).
        id: String,

        /// The name of the tool being invoked.
        name: String,

        /// The input arguments to the tool as a JSON object.
        input: Value,
    },
}

/// User event containing tool execution results.
///
/// After Claude requests a tool use, the tool is executed and the result
/// is sent back as a user event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserEvent {
    /// The message content (typically containing tool results).
    pub message: UserMessage,
}

/// The message payload from a user event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    /// Unique identifier for this message.
    #[serde(default)]
    pub id: Option<String>,

    /// The content blocks in this message (typically tool_result).
    #[serde(default)]
    pub content: Vec<ToolResult>,
}

/// A tool result block within a user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// The type of content (should be "tool_result").
    #[serde(rename = "type", default)]
    pub result_type: Option<String>,

    /// The ID of the tool use this result corresponds to.
    #[serde(default)]
    pub tool_use_id: Option<String>,

    /// The result content from the tool execution.
    #[serde(default)]
    pub content: Option<String>,

    /// Whether the tool execution resulted in an error.
    #[serde(default)]
    pub is_error: bool,
}

/// Result event containing final session costs and usage statistics.
///
/// This event is emitted at the end of a Claude session and contains
/// billing information and token usage metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultEvent {
    /// The subtype of result event (e.g., "success", "error").
    #[serde(default)]
    pub subtype: Option<String>,

    /// Total cost in USD for this session.
    #[serde(default)]
    pub total_cost_usd: Option<f64>,

    /// Alternative field name for total cost.
    #[serde(default)]
    pub cost_usd: Option<f64>,

    /// Duration of the session in milliseconds.
    #[serde(default)]
    pub duration_ms: Option<u64>,

    /// Duration in API time (milliseconds).
    #[serde(default)]
    pub duration_api_ms: Option<u64>,

    /// Token usage statistics.
    #[serde(default)]
    pub usage: Option<Usage>,

    /// Session identifier.
    #[serde(default)]
    pub session_id: Option<String>,

    /// Number of conversation turns.
    #[serde(default)]
    pub num_turns: Option<u32>,

    /// Final result text, if any.
    #[serde(default)]
    pub result: Option<String>,

    #[serde(default)]
    pub terminal_reason: Option<String>,

    #[serde(default, rename = "modelUsage")]
    pub model_usage: BTreeMap<String, ModelTotals>,
}

impl ResultEvent {
    /// Get the total cost in USD, checking both field names.
    pub fn get_cost_usd(&self) -> Option<f64> {
        self.total_cost_usd.or(self.cost_usd)
    }
}

/// Token usage statistics for a Claude session.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Usage {
    /// Number of input tokens used.
    #[serde(default)]
    pub input_tokens: u64,

    /// Number of output tokens generated.
    #[serde(default)]
    pub output_tokens: u64,

    /// Number of tokens read from cache (prompt caching).
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,

    /// Number of tokens written to cache (prompt caching).
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,

    #[serde(default)]
    pub cache_creation: Option<CacheCreation>,

    #[serde(default)]
    pub output_tokens_details: Option<OutputTokensDetails>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CacheCreation {
    #[serde(default)]
    pub ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    pub ephemeral_1h_input_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    #[serde(default)]
    pub thinking_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    #[serde(rename = "cacheCreationEphemeral5mInputTokens")]
    pub cache_write_5m: u64,
    #[serde(rename = "cacheCreationEphemeral1hInputTokens")]
    pub cache_write_1h: u64,
    pub thinking_tokens: u64,
}

impl ModelTotals {
    pub fn saturating_sub(&self, other: &ModelTotals) -> ModelTotals {
        ModelTotals {
            input_tokens: self.input_tokens.saturating_sub(other.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(other.output_tokens),
            cache_read_input_tokens: self
                .cache_read_input_tokens
                .saturating_sub(other.cache_read_input_tokens),
            cache_creation_input_tokens: self
                .cache_creation_input_tokens
                .saturating_sub(other.cache_creation_input_tokens),
            cache_write_5m: self.cache_write_5m.saturating_sub(other.cache_write_5m),
            cache_write_1h: self.cache_write_1h.saturating_sub(other.cache_write_1h),
            thinking_tokens: self.thinking_tokens.saturating_sub(other.thinking_tokens),
        }
    }
}

impl From<&Usage> for ModelTotals {
    fn from(usage: &Usage) -> ModelTotals {
        let tiers = usage.cache_creation.clone().unwrap_or_default();
        ModelTotals {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens.unwrap_or(0),
            cache_creation_input_tokens: usage.cache_creation_input_tokens.unwrap_or(0),
            cache_write_5m: tiers.ephemeral_5m_input_tokens,
            cache_write_1h: tiers.ephemeral_1h_input_tokens,
            thinking_tokens: usage
                .output_tokens_details
                .as_ref()
                .and_then(|details| details.thinking_tokens)
                .unwrap_or(0),
        }
    }
}

impl Usage {
    pub fn context_tokens(&self) -> u64 {
        self.input_tokens
            + self.cache_read_input_tokens.unwrap_or(0)
            + self.cache_creation_input_tokens.unwrap_or(0)
    }
}
