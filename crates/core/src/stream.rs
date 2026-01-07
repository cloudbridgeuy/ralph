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

/// Result of parsing a single line of stream-json output.
#[derive(Debug, Clone)]
pub enum ParsedLine {
    /// Successfully parsed stream event.
    Event(StreamEvent),
    /// Empty line (skipped).
    Empty,
    /// Parse error with the original line and error message.
    Error { line: String, error: String },
}

/// Parse a single line of stream-json output.
///
/// Handles empty lines gracefully (returns `ParsedLine::Empty`) and returns
/// parse errors without crashing (returns `ParsedLine::Error`).
///
/// # Arguments
///
/// * `line` - A single line of stream-json output
///
/// # Returns
///
/// * `ParsedLine::Event` if the line was successfully parsed
/// * `ParsedLine::Empty` if the line was empty or whitespace-only
/// * `ParsedLine::Error` if the line could not be parsed as JSON
///
/// # Example
///
/// ```
/// use ralph_core::stream::{parse_stream_line, ParsedLine, StreamEvent};
///
/// let line = r#"{"type":"system","subtype":"init","session_id":"abc-123"}"#;
/// match parse_stream_line(line) {
///     ParsedLine::Event(event) => {
///         match event {
///             StreamEvent::System(sys) => assert_eq!(sys.session_id, Some("abc-123".to_string())),
///             _ => panic!("Expected system event"),
///         }
///     }
///     _ => panic!("Expected successful parse"),
/// }
/// ```
pub fn parse_stream_line(line: &str) -> ParsedLine {
    let trimmed = line.trim();

    // Handle empty lines
    if trimmed.is_empty() {
        return ParsedLine::Empty;
    }

    // Attempt to parse as JSON
    match serde_json::from_str::<StreamEvent>(trimmed) {
        Ok(event) => ParsedLine::Event(event),
        Err(e) => ParsedLine::Error {
            line: line.to_string(),
            error: e.to_string(),
        },
    }
}

/// Parse multiple lines of stream-json output.
///
/// This is a convenience function for parsing a complete stream-json output
/// that has been captured as a string. It splits on newlines and parses each
/// line, collecting successfully parsed events.
///
/// # Arguments
///
/// * `output` - The complete stream-json output string (newline-delimited JSON)
///
/// # Returns
///
/// A tuple containing:
/// * `Vec<StreamEvent>` - Successfully parsed events in order
/// * `Vec<(usize, String, String)>` - Parse errors as (line_number, original_line, error_message)
///
/// # Example
///
/// ```
/// use ralph_core::stream::{parse_stream_output, StreamEvent};
///
/// let output = r#"{"type":"system","subtype":"init","session_id":"abc"}
/// {"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}
/// {"type":"result","total_cost_usd":0.01}"#;
///
/// let (events, errors) = parse_stream_output(output);
/// assert_eq!(events.len(), 3);
/// assert!(errors.is_empty());
/// ```
pub fn parse_stream_output(output: &str) -> (Vec<StreamEvent>, Vec<(usize, String, String)>) {
    let mut events = Vec::new();
    let mut errors = Vec::new();

    for (line_num, line) in output.lines().enumerate() {
        match parse_stream_line(line) {
            ParsedLine::Event(event) => events.push(event),
            ParsedLine::Empty => {} // Skip empty lines
            ParsedLine::Error {
                line: original,
                error,
            } => {
                errors.push((line_num + 1, original, error));
            }
        }
    }

    (events, errors)
}

/// An iterator that parses stream-json lines on demand.
///
/// This is useful for streaming scenarios where you want to process events
/// as they arrive rather than buffering the entire output.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{StreamParser, ParsedLine, StreamEvent};
///
/// let lines = vec![
///     r#"{"type":"system","session_id":"abc"}"#.to_string(),
///     "".to_string(),  // Empty line, will be skipped
///     r#"{"type":"result","total_cost_usd":0.01}"#.to_string(),
/// ];
///
/// let mut parser = StreamParser::new(lines.into_iter());
/// let mut events = Vec::new();
///
/// for result in parser {
///     match result {
///         ParsedLine::Event(e) => events.push(e),
///         _ => {}
///     }
/// }
///
/// assert_eq!(events.len(), 2);
/// ```
pub struct StreamParser<I>
where
    I: Iterator<Item = String>,
{
    lines: I,
}

impl<I> StreamParser<I>
where
    I: Iterator<Item = String>,
{
    /// Create a new stream parser from an iterator of lines.
    pub fn new(lines: I) -> Self {
        Self { lines }
    }
}

impl<I> Iterator for StreamParser<I>
where
    I: Iterator<Item = String>,
{
    type Item = ParsedLine;

    fn next(&mut self) -> Option<Self::Item> {
        self.lines.next().map(|line| parse_stream_line(&line))
    }
}

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

impl AssistantMessage {
    /// Extract text content from the message.
    ///
    /// Iterates through the `content` array, filters for `ContentBlock::Text` variants,
    /// and concatenates all text fields in order.
    ///
    /// # Returns
    ///
    /// A `String` containing all text content from the message. Returns an empty string
    /// if the message contains no text blocks (e.g., tool-only messages).
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::stream::{AssistantMessage, ContentBlock};
    /// use serde_json::json;
    ///
    /// let message = AssistantMessage {
    ///     id: Some("msg_01".to_string()),
    ///     content: vec![
    ///         ContentBlock::Text { text: "Hello, ".to_string() },
    ///         ContentBlock::ToolUse {
    ///             id: "toolu_01".to_string(),
    ///             name: "Read".to_string(),
    ///             input: json!({}),
    ///         },
    ///         ContentBlock::Text { text: "world!".to_string() },
    ///     ],
    ///     model: None,
    ///     stop_reason: None,
    /// };
    ///
    /// assert_eq!(message.extract_text(), "Hello, world!");
    /// ```
    pub fn extract_text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::ToolUse { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Extract tool invocations from the message.
    ///
    /// Iterates through the `content` array, filters for `ContentBlock::ToolUse` variants,
    /// and returns a list of [`ToolInvocation`] structs containing the tool name, ID, and input.
    ///
    /// # Returns
    ///
    /// A `Vec<ToolInvocation>` containing all tool calls from the message. Returns an empty
    /// vector if the message contains no tool use blocks (e.g., text-only messages).
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::stream::{AssistantMessage, ContentBlock};
    /// use serde_json::json;
    ///
    /// let message = AssistantMessage {
    ///     id: Some("msg_01".to_string()),
    ///     content: vec![
    ///         ContentBlock::Text { text: "Let me read the file.".to_string() },
    ///         ContentBlock::ToolUse {
    ///             id: "toolu_01".to_string(),
    ///             name: "Read".to_string(),
    ///             input: json!({"file_path": "/src/main.rs"}),
    ///         },
    ///     ],
    ///     model: None,
    ///     stop_reason: None,
    /// };
    ///
    /// let invocations = message.extract_tool_invocations();
    /// assert_eq!(invocations.len(), 1);
    /// assert_eq!(invocations[0].name, "Read");
    /// assert_eq!(invocations[0].id, "toolu_01");
    /// ```
    pub fn extract_tool_invocations(&self) -> Vec<ToolInvocation> {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, name, input } => Some(ToolInvocation {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                }),
                ContentBlock::Text { .. } => None,
            })
            .collect()
    }
}

impl AssistantEvent {
    /// Extract text content from the assistant event's message.
    ///
    /// Convenience method that delegates to [`AssistantMessage::extract_text`].
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::stream::{AssistantEvent, AssistantMessage, ContentBlock};
    ///
    /// let event = AssistantEvent {
    ///     message: AssistantMessage {
    ///         id: None,
    ///         content: vec![
    ///             ContentBlock::Text { text: "Hello!".to_string() },
    ///         ],
    ///         model: None,
    ///         stop_reason: None,
    ///     },
    /// };
    ///
    /// assert_eq!(event.extract_text(), "Hello!");
    /// ```
    pub fn extract_text(&self) -> String {
        self.message.extract_text()
    }

    /// Extract tool invocations from the assistant event's message.
    ///
    /// Convenience method that delegates to [`AssistantMessage::extract_tool_invocations`].
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::stream::{AssistantEvent, AssistantMessage, ContentBlock};
    /// use serde_json::json;
    ///
    /// let event = AssistantEvent {
    ///     message: AssistantMessage {
    ///         id: None,
    ///         content: vec![
    ///             ContentBlock::ToolUse {
    ///                 id: "toolu_01".to_string(),
    ///                 name: "Glob".to_string(),
    ///                 input: json!({"pattern": "*.rs"}),
    ///             },
    ///         ],
    ///         model: None,
    ///         stop_reason: None,
    ///     },
    /// };
    ///
    /// let invocations = event.extract_tool_invocations();
    /// assert_eq!(invocations.len(), 1);
    /// assert_eq!(invocations[0].name, "Glob");
    /// ```
    pub fn extract_tool_invocations(&self) -> Vec<ToolInvocation> {
        self.message.extract_tool_invocations()
    }
}

/// Extract text content from a slice of stream events.
///
/// Filters for `StreamEvent::Assistant` variants and concatenates all text content
/// from their messages in order.
///
/// # Arguments
///
/// * `events` - A slice of stream events to extract text from
///
/// # Returns
///
/// A `String` containing all assistant text content. Returns an empty string if there
/// are no assistant events or they contain no text.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{extract_text_from_events, StreamEvent, AssistantEvent, AssistantMessage, ContentBlock, SystemEvent};
///
/// let events = vec![
///     StreamEvent::System(SystemEvent {
///         subtype: Some("init".to_string()),
///         session_id: None,
///         model: None,
///         tools: vec![],
///     }),
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: None,
///             content: vec![ContentBlock::Text { text: "First message. ".to_string() }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: None,
///             content: vec![ContentBlock::Text { text: "Second message.".to_string() }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// assert_eq!(extract_text_from_events(&events), "First message. Second message.");
/// ```
pub fn extract_text_from_events(events: &[StreamEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::Assistant(ast) => Some(ast.extract_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Extract tool invocations from a slice of stream events.
///
/// Filters for `StreamEvent::Assistant` variants and collects all tool invocations
/// from their messages in order.
///
/// # Arguments
///
/// * `events` - A slice of stream events to extract tool invocations from
///
/// # Returns
///
/// A `Vec<ToolInvocation>` containing all tool calls from assistant events. Returns an empty
/// vector if there are no assistant events or they contain no tool use blocks.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{extract_tool_invocations_from_events, StreamEvent, AssistantEvent, AssistantMessage, ContentBlock, SystemEvent};
/// use serde_json::json;
///
/// let events = vec![
///     StreamEvent::System(SystemEvent {
///         subtype: Some("init".to_string()),
///         session_id: None,
///         model: None,
///         tools: vec![],
///     }),
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: None,
///             content: vec![
///                 ContentBlock::Text { text: "Let me search.".to_string() },
///                 ContentBlock::ToolUse {
///                     id: "toolu_01".to_string(),
///                     name: "Glob".to_string(),
///                     input: json!({"pattern": "*.rs"}),
///                 },
///             ],
///             model: None,
///             stop_reason: None,
///         },
///     }),
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: None,
///             content: vec![
///                 ContentBlock::ToolUse {
///                     id: "toolu_02".to_string(),
///                     name: "Read".to_string(),
///                     input: json!({"file_path": "/src/main.rs"}),
///                 },
///             ],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// let invocations = extract_tool_invocations_from_events(&events);
/// assert_eq!(invocations.len(), 2);
/// assert_eq!(invocations[0].name, "Glob");
/// assert_eq!(invocations[1].name, "Read");
/// ```
pub fn extract_tool_invocations_from_events(events: &[StreamEvent]) -> Vec<ToolInvocation> {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::Assistant(ast) => Some(ast.extract_tool_invocations()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// A tool invocation extracted from an assistant event.
///
/// Represents a single tool call from Claude, containing the tool name,
/// unique ID for correlation with results, and input arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInvocation {
    /// Unique identifier for this tool use (used to correlate with results).
    pub id: String,

    /// The name of the tool being invoked (e.g., "Read", "Edit", "Glob").
    pub name: String,

    /// The input arguments to the tool as a JSON object.
    pub input: Value,
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

    // Tests for NDJSON parsing functions (Story #12)

    #[test]
    fn test_parse_stream_line_system_event() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-opus-4-5"}"#;
        match parse_stream_line(line) {
            ParsedLine::Event(StreamEvent::System(sys)) => {
                assert_eq!(sys.session_id, Some("abc-123".to_string()));
                assert_eq!(sys.model, Some("claude-opus-4-5".to_string()));
            }
            other => panic!("Expected System event, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_stream_line_assistant_event() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello, world!"}]}}"#;
        match parse_stream_line(line) {
            ParsedLine::Event(StreamEvent::Assistant(ast)) => {
                assert_eq!(ast.message.content.len(), 1);
                match &ast.message.content[0] {
                    ContentBlock::Text { text } => assert_eq!(text, "Hello, world!"),
                    _ => panic!("Expected Text content"),
                }
            }
            other => panic!("Expected Assistant event, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_stream_line_result_event() {
        let line = r#"{"type":"result","total_cost_usd":0.15,"duration_ms":30000}"#;
        match parse_stream_line(line) {
            ParsedLine::Event(StreamEvent::Result(res)) => {
                assert_eq!(res.total_cost_usd, Some(0.15));
                assert_eq!(res.duration_ms, Some(30000));
            }
            other => panic!("Expected Result event, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_stream_line_empty() {
        assert!(matches!(parse_stream_line(""), ParsedLine::Empty));
        assert!(matches!(parse_stream_line("   "), ParsedLine::Empty));
        assert!(matches!(parse_stream_line("\t\n"), ParsedLine::Empty));
    }

    #[test]
    fn test_parse_stream_line_malformed_json() {
        let line = "this is not json";
        match parse_stream_line(line) {
            ParsedLine::Error {
                line: original,
                error,
            } => {
                assert_eq!(original, "this is not json");
                assert!(!error.is_empty());
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_stream_line_partial_json() {
        let line = r#"{"type":"system""#;
        match parse_stream_line(line) {
            ParsedLine::Error { error, .. } => {
                assert!(!error.is_empty());
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_stream_line_unknown_type() {
        // Unknown type should result in parse error
        let line = r#"{"type":"unknown_event_type"}"#;
        assert!(matches!(parse_stream_line(line), ParsedLine::Error { .. }));
    }

    #[test]
    fn test_parse_stream_line_with_whitespace() {
        let line = r#"  {"type":"system","session_id":"abc"}  "#;
        match parse_stream_line(line) {
            ParsedLine::Event(StreamEvent::System(sys)) => {
                assert_eq!(sys.session_id, Some("abc".to_string()));
            }
            other => panic!("Expected System event, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_stream_output_multiple_events() {
        let output = r#"{"type":"system","session_id":"abc"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}
{"type":"result","total_cost_usd":0.01}"#;

        let (events, errors) = parse_stream_output(output);
        assert_eq!(events.len(), 3);
        assert!(errors.is_empty());

        assert!(matches!(events[0], StreamEvent::System(_)));
        assert!(matches!(events[1], StreamEvent::Assistant(_)));
        assert!(matches!(events[2], StreamEvent::Result(_)));
    }

    #[test]
    fn test_parse_stream_output_with_empty_lines() {
        let output = r#"{"type":"system","session_id":"abc"}

{"type":"result","total_cost_usd":0.01}
"#;

        let (events, errors) = parse_stream_output(output);
        assert_eq!(events.len(), 2);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_parse_stream_output_with_errors() {
        let output = r#"{"type":"system","session_id":"abc"}
not valid json
{"type":"result","total_cost_usd":0.01}"#;

        let (events, errors) = parse_stream_output(output);
        assert_eq!(events.len(), 2);
        assert_eq!(errors.len(), 1);

        let (line_num, original, _error) = &errors[0];
        assert_eq!(*line_num, 2);
        assert_eq!(original, "not valid json");
    }

    #[test]
    fn test_parse_stream_output_empty() {
        let (events, errors) = parse_stream_output("");
        assert!(events.is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn test_parse_stream_output_only_empty_lines() {
        let output = "\n\n\n";
        let (events, errors) = parse_stream_output(output);
        assert!(events.is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn test_parse_stream_output_preserves_order() {
        let output = r#"{"type":"system","subtype":"init"}
{"type":"assistant","message":{"content":[{"type":"text","text":"First"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Second"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Third"}]}}"#;

        let (events, _) = parse_stream_output(output);
        assert_eq!(events.len(), 4);

        // Verify order: system, then three assistants
        assert!(matches!(events[0], StreamEvent::System(_)));

        for (i, expected_text) in [(1, "First"), (2, "Second"), (3, "Third")] {
            match &events[i] {
                StreamEvent::Assistant(ast) => match &ast.message.content[0] {
                    ContentBlock::Text { text } => assert_eq!(text, expected_text),
                    _ => panic!("Expected Text"),
                },
                _ => panic!("Expected Assistant"),
            }
        }
    }

    #[test]
    fn test_stream_parser_iterator() {
        let lines = vec![
            r#"{"type":"system","session_id":"abc"}"#.to_string(),
            "".to_string(),
            r#"{"type":"result","total_cost_usd":0.01}"#.to_string(),
        ];

        let parser = StreamParser::new(lines.into_iter());
        let results: Vec<_> = parser.collect();

        assert_eq!(results.len(), 3);
        assert!(matches!(
            results[0],
            ParsedLine::Event(StreamEvent::System(_))
        ));
        assert!(matches!(results[1], ParsedLine::Empty));
        assert!(matches!(
            results[2],
            ParsedLine::Event(StreamEvent::Result(_))
        ));
    }

    #[test]
    fn test_stream_parser_with_errors() {
        let lines = vec![
            r#"{"type":"system"}"#.to_string(),
            "invalid".to_string(),
            r#"{"type":"result"}"#.to_string(),
        ];

        let parser = StreamParser::new(lines.into_iter());
        let events: Vec<_> = parser
            .filter_map(|r| match r {
                ParsedLine::Event(e) => Some(e),
                _ => None,
            })
            .collect();

        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_stream_parser_empty_iterator() {
        let lines: Vec<String> = vec![];
        let parser = StreamParser::new(lines.into_iter());
        let results: Vec<_> = parser.collect();
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_stream_output_handles_incomplete_line_at_end() {
        // Simulating a stream that might be cut off - the incomplete line should error
        let output = r#"{"type":"system","session_id":"abc"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"#;

        let (events, errors) = parse_stream_output(output);
        assert_eq!(events.len(), 1);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn test_parse_stream_line_user_event() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_123","content":"file contents"}]}}"#;
        match parse_stream_line(line) {
            ParsedLine::Event(StreamEvent::User(usr)) => {
                assert_eq!(usr.message.content.len(), 1);
                assert_eq!(
                    usr.message.content[0].tool_use_id,
                    Some("toolu_123".to_string())
                );
            }
            other => panic!("Expected User event, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_real_claude_output_simulation() {
        // Simulate a realistic Claude stream-json output sequence
        let output = r#"{"type":"system","subtype":"init","session_id":"f5b6aaac-4316-454a","model":"claude-opus-4-5-20251101","tools":[{"name":"Read"},{"name":"Edit"}]}
{"type":"assistant","message":{"id":"msg_01ABC","content":[{"type":"text","text":"I'll help you implement this feature."}],"stop_reason":"end_turn"}}
{"type":"assistant","message":{"id":"msg_01DEF","content":[{"type":"text","text":"Let me read the file first."},{"type":"tool_use","id":"toolu_01XYZ","name":"Read","input":{"file_path":"/src/main.rs"}}],"stop_reason":"tool_use"}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_01XYZ","content":"fn main() { }"}]}}
{"type":"assistant","message":{"id":"msg_01GHI","content":[{"type":"text","text":"Done! The implementation is complete."}],"stop_reason":"end_turn"}}
{"type":"result","subtype":"success","total_cost_usd":0.226354,"duration_ms":40966,"num_turns":3,"usage":{"input_tokens":712,"output_tokens":2971}}"#;

        let (events, errors) = parse_stream_output(output);

        assert!(errors.is_empty(), "Parse errors: {:?}", errors);
        assert_eq!(events.len(), 6);

        // Verify the sequence
        match &events[0] {
            StreamEvent::System(sys) => {
                assert_eq!(sys.session_id, Some("f5b6aaac-4316-454a".to_string()));
                assert_eq!(sys.tools.len(), 2);
            }
            _ => panic!("Expected System"),
        }

        // Check result event
        match &events[5] {
            StreamEvent::Result(res) => {
                assert_eq!(res.total_cost_usd, Some(0.226354));
                assert_eq!(res.duration_ms, Some(40966));
                assert_eq!(res.num_turns, Some(3));
            }
            _ => panic!("Expected Result"),
        }
    }

    // Tests for text extraction (Story #13)

    #[test]
    fn test_assistant_message_extract_text_single_content() {
        let message = AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Hello, world!".to_string(),
            }],
            model: None,
            stop_reason: None,
        };

        assert_eq!(message.extract_text(), "Hello, world!");
    }

    #[test]
    fn test_assistant_message_extract_text_multi_content() {
        let message = AssistantMessage {
            id: None,
            content: vec![
                ContentBlock::Text {
                    text: "First part. ".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({"file_path": "/test.rs"}),
                },
                ContentBlock::Text {
                    text: "Second part.".to_string(),
                },
            ],
            model: None,
            stop_reason: None,
        };

        assert_eq!(message.extract_text(), "First part. Second part.");
    }

    #[test]
    fn test_assistant_message_extract_text_tool_only() {
        let message = AssistantMessage {
            id: None,
            content: vec![
                ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Glob".to_string(),
                    input: serde_json::json!({"pattern": "*.rs"}),
                },
                ContentBlock::ToolUse {
                    id: "toolu_02".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({"file_path": "/test.rs"}),
                },
            ],
            model: None,
            stop_reason: None,
        };

        assert_eq!(message.extract_text(), "");
    }

    #[test]
    fn test_assistant_message_extract_text_empty_content() {
        let message = AssistantMessage {
            id: None,
            content: vec![],
            model: None,
            stop_reason: None,
        };

        assert_eq!(message.extract_text(), "");
    }

    #[test]
    fn test_assistant_message_extract_text_preserves_ordering() {
        let message = AssistantMessage {
            id: None,
            content: vec![
                ContentBlock::Text {
                    text: "A".to_string(),
                },
                ContentBlock::Text {
                    text: "B".to_string(),
                },
                ContentBlock::Text {
                    text: "C".to_string(),
                },
            ],
            model: None,
            stop_reason: None,
        };

        assert_eq!(message.extract_text(), "ABC");
    }

    #[test]
    fn test_assistant_event_extract_text() {
        let event = AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Hello from assistant event!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        };

        assert_eq!(event.extract_text(), "Hello from assistant event!");
    }

    #[test]
    fn test_extract_text_from_events_assistant_only() {
        let events = vec![
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![ContentBlock::Text {
                        text: "First.".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![ContentBlock::Text {
                        text: "Second.".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
        ];

        assert_eq!(extract_text_from_events(&events), "First.Second.");
    }

    #[test]
    fn test_extract_text_from_events_mixed() {
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: Some("abc".to_string()),
                model: None,
                tools: vec![],
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![ContentBlock::Text {
                        text: "Hello! ".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::User(UserEvent {
                message: UserMessage {
                    id: None,
                    content: vec![ToolResult {
                        result_type: Some("tool_result".to_string()),
                        tool_use_id: Some("toolu_01".to_string()),
                        content: Some("file contents".to_string()),
                        is_error: false,
                    }],
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![ContentBlock::Text {
                        text: "Done!".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Result(ResultEvent {
                subtype: Some("success".to_string()),
                total_cost_usd: Some(0.01),
                cost_usd: None,
                duration_ms: Some(1000),
                duration_api_ms: None,
                usage: None,
                session_id: None,
                num_turns: None,
                result: None,
            }),
        ];

        assert_eq!(extract_text_from_events(&events), "Hello! Done!");
    }

    #[test]
    fn test_extract_text_from_events_empty() {
        let events: Vec<StreamEvent> = vec![];
        assert_eq!(extract_text_from_events(&events), "");
    }

    #[test]
    fn test_extract_text_from_events_no_assistant() {
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: None,
                session_id: None,
                model: None,
                tools: vec![],
            }),
            StreamEvent::Result(ResultEvent {
                subtype: None,
                total_cost_usd: None,
                cost_usd: None,
                duration_ms: None,
                duration_api_ms: None,
                usage: None,
                session_id: None,
                num_turns: None,
                result: None,
            }),
        ];

        assert_eq!(extract_text_from_events(&events), "");
    }

    #[test]
    fn test_extract_text_from_events_tool_only_assistant() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({}),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        assert_eq!(extract_text_from_events(&events), "");
    }

    // Tests for tool invocation extraction (Story #14)

    #[test]
    fn test_assistant_message_extract_tool_invocations_single() {
        let message = AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"file_path": "/src/main.rs"}),
            }],
            model: None,
            stop_reason: None,
        };

        let invocations = message.extract_tool_invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].id, "toolu_01");
        assert_eq!(invocations[0].name, "Read");
        assert_eq!(invocations[0].input["file_path"], "/src/main.rs");
    }

    #[test]
    fn test_assistant_message_extract_tool_invocations_multiple() {
        let message = AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![
                ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Glob".to_string(),
                    input: serde_json::json!({"pattern": "*.rs"}),
                },
                ContentBlock::ToolUse {
                    id: "toolu_02".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({"file_path": "/src/lib.rs"}),
                },
            ],
            model: None,
            stop_reason: None,
        };

        let invocations = message.extract_tool_invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].id, "toolu_01");
        assert_eq!(invocations[0].name, "Glob");
        assert_eq!(invocations[1].id, "toolu_02");
        assert_eq!(invocations[1].name, "Read");
    }

    #[test]
    fn test_assistant_message_extract_tool_invocations_mixed_content() {
        let message = AssistantMessage {
            id: None,
            content: vec![
                ContentBlock::Text {
                    text: "Let me search for files.".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Glob".to_string(),
                    input: serde_json::json!({"pattern": "**/*.rs"}),
                },
                ContentBlock::Text {
                    text: "Now reading the file.".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "toolu_02".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({"file_path": "/test.rs"}),
                },
            ],
            model: None,
            stop_reason: None,
        };

        let invocations = message.extract_tool_invocations();
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].name, "Glob");
        assert_eq!(invocations[1].name, "Read");
    }

    #[test]
    fn test_assistant_message_extract_tool_invocations_text_only() {
        let message = AssistantMessage {
            id: None,
            content: vec![
                ContentBlock::Text {
                    text: "Hello, world!".to_string(),
                },
                ContentBlock::Text {
                    text: " More text.".to_string(),
                },
            ],
            model: None,
            stop_reason: None,
        };

        let invocations = message.extract_tool_invocations();
        assert!(invocations.is_empty());
    }

    #[test]
    fn test_assistant_message_extract_tool_invocations_empty_content() {
        let message = AssistantMessage {
            id: None,
            content: vec![],
            model: None,
            stop_reason: None,
        };

        let invocations = message.extract_tool_invocations();
        assert!(invocations.is_empty());
    }

    #[test]
    fn test_assistant_message_extract_tool_invocations_preserves_input() {
        let complex_input = serde_json::json!({
            "file_path": "/src/main.rs",
            "old_string": "fn main() {}",
            "new_string": "fn main() { println!(\"Hello!\"); }",
            "nested": {
                "key": "value",
                "array": [1, 2, 3]
            }
        });

        let message = AssistantMessage {
            id: None,
            content: vec![ContentBlock::ToolUse {
                id: "toolu_edit".to_string(),
                name: "Edit".to_string(),
                input: complex_input.clone(),
            }],
            model: None,
            stop_reason: None,
        };

        let invocations = message.extract_tool_invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].input, complex_input);
        assert_eq!(invocations[0].input["nested"]["array"][1], 2);
    }

    #[test]
    fn test_assistant_event_extract_tool_invocations() {
        let event = AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Glob".to_string(),
                    input: serde_json::json!({"pattern": "*.rs"}),
                }],
                model: None,
                stop_reason: None,
            },
        };

        let invocations = event.extract_tool_invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].name, "Glob");
    }

    #[test]
    fn test_extract_tool_invocations_from_events_single_assistant() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({"file_path": "/test.rs"}),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let invocations = extract_tool_invocations_from_events(&events);
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].name, "Read");
    }

    #[test]
    fn test_extract_tool_invocations_from_events_multiple_assistants() {
        let events = vec![
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![ContentBlock::ToolUse {
                        id: "toolu_01".to_string(),
                        name: "Glob".to_string(),
                        input: serde_json::json!({"pattern": "*.rs"}),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![ContentBlock::ToolUse {
                        id: "toolu_02".to_string(),
                        name: "Read".to_string(),
                        input: serde_json::json!({"file_path": "/src/main.rs"}),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
        ];

        let invocations = extract_tool_invocations_from_events(&events);
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].name, "Glob");
        assert_eq!(invocations[0].id, "toolu_01");
        assert_eq!(invocations[1].name, "Read");
        assert_eq!(invocations[1].id, "toolu_02");
    }

    #[test]
    fn test_extract_tool_invocations_from_events_mixed_event_types() {
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: Some("abc".to_string()),
                model: None,
                tools: vec![],
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![
                        ContentBlock::Text {
                            text: "Searching...".to_string(),
                        },
                        ContentBlock::ToolUse {
                            id: "toolu_01".to_string(),
                            name: "Glob".to_string(),
                            input: serde_json::json!({"pattern": "*.rs"}),
                        },
                    ],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::User(UserEvent {
                message: UserMessage {
                    id: None,
                    content: vec![ToolResult {
                        result_type: Some("tool_result".to_string()),
                        tool_use_id: Some("toolu_01".to_string()),
                        content: Some("/src/main.rs".to_string()),
                        is_error: false,
                    }],
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![ContentBlock::ToolUse {
                        id: "toolu_02".to_string(),
                        name: "Read".to_string(),
                        input: serde_json::json!({"file_path": "/src/main.rs"}),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Result(ResultEvent {
                subtype: Some("success".to_string()),
                total_cost_usd: Some(0.01),
                cost_usd: None,
                duration_ms: Some(1000),
                duration_api_ms: None,
                usage: None,
                session_id: None,
                num_turns: None,
                result: None,
            }),
        ];

        let invocations = extract_tool_invocations_from_events(&events);
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].name, "Glob");
        assert_eq!(invocations[1].name, "Read");
    }

    #[test]
    fn test_extract_tool_invocations_from_events_empty() {
        let events: Vec<StreamEvent> = vec![];
        let invocations = extract_tool_invocations_from_events(&events);
        assert!(invocations.is_empty());
    }

    #[test]
    fn test_extract_tool_invocations_from_events_no_assistant() {
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: None,
                session_id: None,
                model: None,
                tools: vec![],
            }),
            StreamEvent::Result(ResultEvent {
                subtype: None,
                total_cost_usd: None,
                cost_usd: None,
                duration_ms: None,
                duration_api_ms: None,
                usage: None,
                session_id: None,
                num_turns: None,
                result: None,
            }),
        ];

        let invocations = extract_tool_invocations_from_events(&events);
        assert!(invocations.is_empty());
    }

    #[test]
    fn test_extract_tool_invocations_from_events_text_only_assistants() {
        let events = vec![
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![ContentBlock::Text {
                        text: "Hello!".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: None,
                    content: vec![ContentBlock::Text {
                        text: "Done!".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
        ];

        let invocations = extract_tool_invocations_from_events(&events);
        assert!(invocations.is_empty());
    }

    #[test]
    fn test_extract_tool_invocations_preserves_order() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![
                    ContentBlock::ToolUse {
                        id: "toolu_a".to_string(),
                        name: "Glob".to_string(),
                        input: serde_json::json!({}),
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_b".to_string(),
                        name: "Read".to_string(),
                        input: serde_json::json!({}),
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_c".to_string(),
                        name: "Edit".to_string(),
                        input: serde_json::json!({}),
                    },
                ],
                model: None,
                stop_reason: None,
            },
        })];

        let invocations = extract_tool_invocations_from_events(&events);
        assert_eq!(invocations.len(), 3);
        assert_eq!(invocations[0].id, "toolu_a");
        assert_eq!(invocations[1].id, "toolu_b");
        assert_eq!(invocations[2].id, "toolu_c");
    }

    #[test]
    fn test_tool_invocation_serialization_round_trip() {
        let invocation = ToolInvocation {
            id: "toolu_01".to_string(),
            name: "Read".to_string(),
            input: serde_json::json!({"file_path": "/src/main.rs", "limit": 100}),
        };

        let json = serde_json::to_string(&invocation).unwrap();
        let deserialized: ToolInvocation = serde_json::from_str(&json).unwrap();
        assert_eq!(invocation, deserialized);
    }
}
