//! Stream-JSON event types for Claude CLI output parsing.
//!
//! This module defines Rust types that correspond to Claude CLI's `--output-format stream-json`
//! output. The events are newline-delimited JSON (NDJSON) that describe the LLM conversation
//! including system initialization, assistant responses, tool interactions, and final results.
//!
//! # Event Types
//!
//! - [`StreamEvent::System`] - Session initialization with metadata
//! - [`StreamEvent::Assistant`] - LLM responses containing text and/or tool calls
//! - [`StreamEvent::User`] - Tool execution results
//! - [`StreamEvent::Result`] - Final costs, usage, and duration
//!
//! # Example
//!
//! ```
//! use ralph_core::stream::StreamEvent;
//!
//! let json = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-opus-4-5"}"#;
//! let event: StreamEvent = serde_json::from_str(json).unwrap();
//!
//! match event {
//!     StreamEvent::System(sys) => {
//!         assert_eq!(sys.session_id, Some("abc-123".to_string()));
//!     }
//!     _ => panic!("Expected system event"),
//! }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
/// - `"init"` - Session initialization with session_id, model, and tools
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

    /// List of tools available in this session.
    #[serde(default)]
    pub tools: Vec<Tool>,
}

/// A tool available in the Claude session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    /// The name of the tool (e.g., "Glob", "Read", "Edit").
    pub name: String,

    /// Tool description, if available.
    #[serde(default)]
    pub description: Option<String>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_init_event() {
        let json = r#"{
            "type": "system",
            "subtype": "init",
            "session_id": "f5b6aaac-4316-454a-b086-a3f9e4351b1e",
            "model": "claude-opus-4-5-20251101",
            "tools": [
                {"name": "Glob", "description": "Find files matching pattern"},
                {"name": "Read"}
            ]
        }"#;

        let event: StreamEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamEvent::System(sys) => {
                assert_eq!(sys.subtype, Some("init".to_string()));
                assert_eq!(
                    sys.session_id,
                    Some("f5b6aaac-4316-454a-b086-a3f9e4351b1e".to_string())
                );
                assert_eq!(sys.model, Some("claude-opus-4-5-20251101".to_string()));
                assert_eq!(sys.tools.len(), 2);
                assert_eq!(sys.tools[0].name, "Glob");
                assert_eq!(
                    sys.tools[0].description,
                    Some("Find files matching pattern".to_string())
                );
                assert_eq!(sys.tools[1].name, "Read");
                assert_eq!(sys.tools[1].description, None);
            }
            _ => panic!("Expected System event"),
        }
    }

    #[test]
    fn test_assistant_text_event() {
        let json = r#"{
            "type": "assistant",
            "message": {
                "id": "msg_01ABC",
                "content": [
                    {"type": "text", "text": "I'll help you implement this feature."}
                ],
                "model": "claude-opus-4-5-20251101",
                "stop_reason": "end_turn"
            }
        }"#;

        let event: StreamEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamEvent::Assistant(ast) => {
                assert_eq!(ast.message.id, Some("msg_01ABC".to_string()));
                assert_eq!(ast.message.content.len(), 1);
                match &ast.message.content[0] {
                    ContentBlock::Text { text } => {
                        assert_eq!(text, "I'll help you implement this feature.");
                    }
                    _ => panic!("Expected Text content"),
                }
                assert_eq!(ast.message.stop_reason, Some("end_turn".to_string()));
            }
            _ => panic!("Expected Assistant event"),
        }
    }

    #[test]
    fn test_assistant_tool_use_event() {
        let json = r#"{
            "type": "assistant",
            "message": {
                "id": "msg_01DEF",
                "content": [
                    {"type": "text", "text": "Let me search for files."},
                    {
                        "type": "tool_use",
                        "id": "toolu_01YWLzHW2VBHQSz8VV1oCGSp",
                        "name": "Glob",
                        "input": {"pattern": ".github/workflows/*.yml"}
                    }
                ],
                "stop_reason": "tool_use"
            }
        }"#;

        let event: StreamEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamEvent::Assistant(ast) => {
                assert_eq!(ast.message.content.len(), 2);
                match &ast.message.content[0] {
                    ContentBlock::Text { text } => {
                        assert_eq!(text, "Let me search for files.");
                    }
                    _ => panic!("Expected Text content"),
                }
                match &ast.message.content[1] {
                    ContentBlock::ToolUse { id, name, input } => {
                        assert_eq!(id, "toolu_01YWLzHW2VBHQSz8VV1oCGSp");
                        assert_eq!(name, "Glob");
                        assert_eq!(input["pattern"], ".github/workflows/*.yml");
                    }
                    _ => panic!("Expected ToolUse content"),
                }
            }
            _ => panic!("Expected Assistant event"),
        }
    }

    #[test]
    fn test_user_tool_result_event() {
        let json = r#"{
            "type": "user",
            "message": {
                "id": "msg_02GHI",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_01YWLzHW2VBHQSz8VV1oCGSp",
                        "content": "/Users/dev/project/.github/workflows/ci.yml\n/Users/dev/project/.github/workflows/release.yml"
                    }
                ]
            }
        }"#;

        let event: StreamEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamEvent::User(usr) => {
                assert_eq!(usr.message.id, Some("msg_02GHI".to_string()));
                assert_eq!(usr.message.content.len(), 1);
                let result = &usr.message.content[0];
                assert_eq!(result.result_type, Some("tool_result".to_string()));
                assert_eq!(
                    result.tool_use_id,
                    Some("toolu_01YWLzHW2VBHQSz8VV1oCGSp".to_string())
                );
                assert!(result.content.as_ref().unwrap().contains("ci.yml"));
                assert!(!result.is_error);
            }
            _ => panic!("Expected User event"),
        }
    }

    #[test]
    fn test_user_tool_error_result() {
        let json = r#"{
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "toolu_error",
                        "content": "Permission denied",
                        "is_error": true
                    }
                ]
            }
        }"#;

        let event: StreamEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamEvent::User(usr) => {
                let result = &usr.message.content[0];
                assert!(result.is_error);
                assert_eq!(result.content, Some("Permission denied".to_string()));
            }
            _ => panic!("Expected User event"),
        }
    }

    #[test]
    fn test_result_event() {
        let json = r#"{
            "type": "result",
            "subtype": "success",
            "total_cost_usd": 0.226354,
            "duration_ms": 40966,
            "session_id": "f5b6aaac-4316-454a-b086-a3f9e4351b1e",
            "num_turns": 5,
            "usage": {
                "input_tokens": 712,
                "output_tokens": 2971,
                "cache_read_input_tokens": 107476,
                "cache_creation_input_tokens": 12504
            }
        }"#;

        let event: StreamEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamEvent::Result(res) => {
                assert_eq!(res.subtype, Some("success".to_string()));
                assert_eq!(res.total_cost_usd, Some(0.226354));
                assert_eq!(res.get_cost_usd(), Some(0.226354));
                assert_eq!(res.duration_ms, Some(40966));
                assert_eq!(
                    res.session_id,
                    Some("f5b6aaac-4316-454a-b086-a3f9e4351b1e".to_string())
                );
                assert_eq!(res.num_turns, Some(5));

                let usage = res.usage.unwrap();
                assert_eq!(usage.input_tokens, 712);
                assert_eq!(usage.output_tokens, 2971);
                assert_eq!(usage.cache_read_input_tokens, Some(107476));
                assert_eq!(usage.cache_creation_input_tokens, Some(12504));
            }
            _ => panic!("Expected Result event"),
        }
    }

    #[test]
    fn test_result_event_alternative_cost_field() {
        let json = r#"{
            "type": "result",
            "cost_usd": 0.15,
            "duration_ms": 30000
        }"#;

        let event: StreamEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamEvent::Result(res) => {
                assert_eq!(res.cost_usd, Some(0.15));
                assert_eq!(res.total_cost_usd, None);
                assert_eq!(res.get_cost_usd(), Some(0.15));
            }
            _ => panic!("Expected Result event"),
        }
    }

    #[test]
    fn test_empty_content_arrays() {
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": []
            }
        }"#;

        let event: StreamEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamEvent::Assistant(ast) => {
                assert!(ast.message.content.is_empty());
            }
            _ => panic!("Expected Assistant event"),
        }
    }

    #[test]
    fn test_missing_optional_fields() {
        let json = r#"{
            "type": "system"
        }"#;

        let event: StreamEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamEvent::System(sys) => {
                assert_eq!(sys.subtype, None);
                assert_eq!(sys.session_id, None);
                assert_eq!(sys.model, None);
                assert!(sys.tools.is_empty());
            }
            _ => panic!("Expected System event"),
        }
    }

    #[test]
    fn test_usage_defaults() {
        let usage = Usage::default();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.cache_read_input_tokens, None);
        assert_eq!(usage.cache_creation_input_tokens, None);
    }

    #[test]
    fn test_round_trip_serialization() {
        let original = StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("test-session".to_string()),
            model: Some("claude-opus-4-5-20251101".to_string()),
            tools: vec![Tool {
                name: "Read".to_string(),
                description: Some("Read a file".to_string()),
            }],
        });

        let json = serde_json::to_string(&original).unwrap();
        let deserialized: StreamEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_content_block_text_round_trip() {
        let block = ContentBlock::Text {
            text: "Hello, world!".to_string(),
        };
        let json = serde_json::to_string(&block).unwrap();
        let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, deserialized);
    }

    #[test]
    fn test_content_block_tool_use_round_trip() {
        let block = ContentBlock::ToolUse {
            id: "toolu_123".to_string(),
            name: "Edit".to_string(),
            input: serde_json::json!({
                "file_path": "/src/main.rs",
                "old_string": "foo",
                "new_string": "bar"
            }),
        };
        let json = serde_json::to_string(&block).unwrap();
        let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, deserialized);
    }
}
