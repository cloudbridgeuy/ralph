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

/// Accumulates text across streaming assistant events.
///
/// In Claude's stream-json output, a single logical response may be split across
/// multiple assistant events. This accumulator tracks text by message ID, starting
/// fresh accumulation when a new message ID is encountered.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{TextAccumulator, StreamEvent, AssistantEvent, AssistantMessage, ContentBlock};
///
/// let mut accumulator = TextAccumulator::new();
///
/// // First event with message ID "msg_01"
/// let event1 = StreamEvent::Assistant(AssistantEvent {
///     message: AssistantMessage {
///         id: Some("msg_01".to_string()),
///         content: vec![ContentBlock::Text { text: "Hello, ".to_string() }],
///         model: None,
///         stop_reason: None,
///     },
/// });
///
/// // Second event with same message ID (continuation)
/// let event2 = StreamEvent::Assistant(AssistantEvent {
///     message: AssistantMessage {
///         id: Some("msg_01".to_string()),
///         content: vec![ContentBlock::Text { text: "world!".to_string() }],
///         model: None,
///         stop_reason: None,
///     },
/// });
///
/// accumulator.process_event(&event1);
/// accumulator.process_event(&event2);
///
/// assert_eq!(accumulator.get_text(), "Hello, world!");
/// ```
#[derive(Debug, Clone, Default)]
pub struct TextAccumulator {
    /// The current message ID being accumulated.
    current_message_id: Option<String>,
    /// Buffer for accumulated text.
    buffer: String,
    /// Completed messages (message_id -> text).
    completed: Vec<AccumulatedMessage>,
}

/// A completed accumulated message.
#[derive(Debug, Clone, PartialEq)]
pub struct AccumulatedMessage {
    /// The message ID (if available).
    pub id: Option<String>,
    /// The accumulated text content.
    pub text: String,
}

impl TextAccumulator {
    /// Create a new empty text accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a stream event, accumulating text from assistant events.
    ///
    /// - If the event is an assistant event, extracts text and appends to buffer.
    /// - If the message ID changes, the previous message is marked complete.
    /// - Non-assistant events are ignored.
    ///
    /// # Arguments
    ///
    /// * `event` - The stream event to process
    ///
    /// # Returns
    ///
    /// `Some(AccumulatedMessage)` if a message was completed (new ID detected),
    /// `None` otherwise.
    pub fn process_event(&mut self, event: &StreamEvent) -> Option<AccumulatedMessage> {
        let StreamEvent::Assistant(assistant) = event else {
            return None;
        };

        let message_id = assistant.message.id.clone();
        let text = assistant.extract_text();

        // Check if this is a new message (different ID)
        let completed = if message_id != self.current_message_id && !self.buffer.is_empty() {
            // Complete the previous message
            let completed_message = AccumulatedMessage {
                id: self.current_message_id.take(),
                text: std::mem::take(&mut self.buffer),
            };
            Some(completed_message)
        } else {
            None
        };

        // Update current message ID and append text
        self.current_message_id = message_id;
        self.buffer.push_str(&text);

        // Store completed message if any
        if let Some(ref msg) = completed {
            self.completed.push(msg.clone());
        }

        completed
    }

    /// Finalize accumulation and return any remaining text.
    ///
    /// Call this when the stream ends to get the last message.
    ///
    /// # Returns
    ///
    /// `Some(AccumulatedMessage)` if there's remaining text in the buffer,
    /// `None` if the buffer is empty.
    pub fn finish(&mut self) -> Option<AccumulatedMessage> {
        if self.buffer.is_empty() {
            return None;
        }

        let completed_message = AccumulatedMessage {
            id: self.current_message_id.take(),
            text: std::mem::take(&mut self.buffer),
        };

        self.completed.push(completed_message.clone());
        Some(completed_message)
    }

    /// Get the current accumulated text (without finishing).
    ///
    /// This returns the text currently in the buffer without marking
    /// the message as complete.
    pub fn get_text(&self) -> &str {
        &self.buffer
    }

    /// Get all completed messages so far.
    ///
    /// This includes messages that were completed when a new message ID
    /// was detected, plus any message completed via `finish()`.
    pub fn completed_messages(&self) -> &[AccumulatedMessage] {
        &self.completed
    }

    /// Get all text from all messages (completed + current buffer).
    ///
    /// This is useful for getting the final combined text after processing
    /// all events, matching what would appear in plain-text mode.
    pub fn get_all_text(&self) -> String {
        let mut result = String::new();
        for msg in &self.completed {
            result.push_str(&msg.text);
        }
        result.push_str(&self.buffer);
        result
    }

    /// Reset the accumulator to its initial state.
    pub fn reset(&mut self) {
        self.current_message_id = None;
        self.buffer.clear();
        self.completed.clear();
    }
}

/// Accumulate text from a sequence of stream events.
///
/// This is a convenience function that processes all events through a
/// [`TextAccumulator`] and returns the final combined text.
///
/// # Arguments
///
/// * `events` - Iterator of stream events to process
///
/// # Returns
///
/// The accumulated text from all assistant events, matching what would
/// appear in Claude's plain-text output mode.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{accumulate_text, StreamEvent, AssistantEvent, AssistantMessage, ContentBlock, SystemEvent};
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
///             id: Some("msg_01".to_string()),
///             content: vec![ContentBlock::Text { text: "Hello, ".to_string() }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: Some("msg_01".to_string()),
///             content: vec![ContentBlock::Text { text: "world!".to_string() }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// let text = accumulate_text(events.iter());
/// assert_eq!(text, "Hello, world!");
/// ```
pub fn accumulate_text<'a>(events: impl Iterator<Item = &'a StreamEvent>) -> String {
    let mut accumulator = TextAccumulator::new();
    for event in events {
        accumulator.process_event(event);
    }
    accumulator.finish();
    accumulator.get_all_text()
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

/// Metadata extracted from a system init event.
///
/// Contains session metadata from Claude's initialization event, including
/// session ID, model name, and available tools.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{extract_metadata_from_events, StreamEvent, SystemEvent, Tool};
///
/// let events = vec![
///     StreamEvent::System(SystemEvent {
///         subtype: Some("init".to_string()),
///         session_id: Some("abc-123".to_string()),
///         model: Some("claude-opus-4-5-20251101".to_string()),
///         tools: vec![
///             Tool { name: "Read".to_string(), description: Some("Read files".to_string()) },
///         ],
///     }),
/// ];
///
/// let metadata = extract_metadata_from_events(&events);
/// assert!(metadata.is_some());
/// let meta = metadata.unwrap();
/// assert_eq!(meta.session_id, Some("abc-123".to_string()));
/// assert_eq!(meta.model, Some("claude-opus-4-5-20251101".to_string()));
/// assert_eq!(meta.tools.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IterationMetadata {
    /// Unique identifier for this Claude session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// The Claude model being used (e.g., "claude-opus-4-5-20251101").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// List of tools available in this session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
}

impl IterationMetadata {
    /// Create a new empty metadata struct.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if any metadata fields are populated.
    pub fn is_empty(&self) -> bool {
        self.session_id.is_none() && self.model.is_none() && self.tools.is_empty()
    }
}

impl SystemEvent {
    /// Check if this is an init event (subtype is "init").
    pub fn is_init(&self) -> bool {
        self.subtype.as_deref() == Some("init")
    }

    /// Extract metadata from this system event.
    ///
    /// Creates an [`IterationMetadata`] struct from the system event fields.
    /// This works for any system event, but is most useful for init events.
    ///
    /// # Returns
    ///
    /// An [`IterationMetadata`] populated with the session_id, model, and tools
    /// from this system event.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::stream::{SystemEvent, Tool};
    ///
    /// let event = SystemEvent {
    ///     subtype: Some("init".to_string()),
    ///     session_id: Some("session-123".to_string()),
    ///     model: Some("claude-opus-4-5".to_string()),
    ///     tools: vec![Tool { name: "Read".to_string(), description: None }],
    /// };
    ///
    /// let metadata = event.extract_metadata();
    /// assert_eq!(metadata.session_id, Some("session-123".to_string()));
    /// assert_eq!(metadata.model, Some("claude-opus-4-5".to_string()));
    /// assert_eq!(metadata.tools.len(), 1);
    /// ```
    pub fn extract_metadata(&self) -> IterationMetadata {
        IterationMetadata {
            session_id: self.session_id.clone(),
            model: self.model.clone(),
            tools: self.tools.clone(),
        }
    }
}

/// Extract metadata from a slice of stream events.
///
/// Searches for the first system init event and extracts metadata from it.
/// Returns `None` if no system init event is found.
///
/// # Arguments
///
/// * `events` - A slice of stream events to search
///
/// # Returns
///
/// `Some(IterationMetadata)` if a system init event was found, `None` otherwise.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     extract_metadata_from_events, StreamEvent, SystemEvent, AssistantEvent,
///     AssistantMessage, ContentBlock, Tool,
/// };
///
/// let events = vec![
///     StreamEvent::System(SystemEvent {
///         subtype: Some("init".to_string()),
///         session_id: Some("f5b6aaac-4316-454a".to_string()),
///         model: Some("claude-opus-4-5-20251101".to_string()),
///         tools: vec![
///             Tool { name: "Glob".to_string(), description: Some("Find files".to_string()) },
///             Tool { name: "Read".to_string(), description: None },
///         ],
///     }),
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: Some("msg_01".to_string()),
///             content: vec![ContentBlock::Text { text: "Hello!".to_string() }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// let metadata = extract_metadata_from_events(&events);
/// assert!(metadata.is_some());
/// let meta = metadata.unwrap();
/// assert_eq!(meta.session_id, Some("f5b6aaac-4316-454a".to_string()));
/// assert_eq!(meta.model, Some("claude-opus-4-5-20251101".to_string()));
/// assert_eq!(meta.tools.len(), 2);
/// assert_eq!(meta.tools[0].name, "Glob");
/// assert_eq!(meta.tools[1].name, "Read");
/// ```
pub fn extract_metadata_from_events(events: &[StreamEvent]) -> Option<IterationMetadata> {
    events.iter().find_map(|event| {
        if let StreamEvent::System(sys) = event {
            if sys.is_init() {
                return Some(sys.extract_metadata());
            }
        }
        None
    })
}

/// Extract metadata from a slice of stream events, returning default if not found.
///
/// Similar to [`extract_metadata_from_events`], but returns a default (empty)
/// [`IterationMetadata`] instead of `None` when no init event is found.
/// This is useful when you want to proceed with default values.
///
/// # Arguments
///
/// * `events` - A slice of stream events to search
///
/// # Returns
///
/// An [`IterationMetadata`] populated from the init event, or a default empty struct.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     extract_metadata_from_events_or_default, StreamEvent, AssistantEvent,
///     AssistantMessage, ContentBlock,
/// };
///
/// // No system event present - returns default
/// let events = vec![
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: None,
///             content: vec![ContentBlock::Text { text: "Hello!".to_string() }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// let metadata = extract_metadata_from_events_or_default(&events);
/// assert!(metadata.is_empty());
/// assert_eq!(metadata.session_id, None);
/// assert_eq!(metadata.model, None);
/// assert!(metadata.tools.is_empty());
/// ```
pub fn extract_metadata_from_events_or_default(events: &[StreamEvent]) -> IterationMetadata {
    extract_metadata_from_events(events).unwrap_or_default()
}

/// Cost and usage statistics extracted from a result event.
///
/// Contains billing information and token usage metrics from Claude's final
/// result event. Used for tracking costs across iterations and sessions.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{extract_costs_from_events, StreamEvent, ResultEvent, Usage};
///
/// let events = vec![
///     StreamEvent::Result(ResultEvent {
///         subtype: Some("success".to_string()),
///         total_cost_usd: Some(0.226354),
///         cost_usd: None,
///         duration_ms: Some(40966),
///         duration_api_ms: None,
///         usage: Some(Usage {
///             input_tokens: 712,
///             output_tokens: 2971,
///             cache_read_input_tokens: Some(107476),
///             cache_creation_input_tokens: Some(12504),
///         }),
///         session_id: None,
///         num_turns: Some(3),
///         result: None,
///     }),
/// ];
///
/// let costs = extract_costs_from_events(&events);
/// assert!(costs.is_some());
/// let c = costs.unwrap();
/// assert_eq!(c.cost_usd, Some(0.226354));
/// assert_eq!(c.duration_ms, Some(40966));
/// assert!(c.usage.is_some());
/// let usage = c.usage.unwrap();
/// assert_eq!(usage.input_tokens, 712);
/// assert_eq!(usage.output_tokens, 2971);
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct IterationCosts {
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

impl IterationCosts {
    /// Create a new empty costs struct.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if any cost/usage fields are populated.
    pub fn is_empty(&self) -> bool {
        self.cost_usd.is_none() && self.duration_ms.is_none() && self.usage.is_none()
    }
}

impl ResultEvent {
    /// Extract cost and usage statistics from this result event.
    ///
    /// Creates an [`IterationCosts`] struct from the result event fields.
    /// The cost is extracted using [`ResultEvent::get_cost_usd`] which
    /// handles both `total_cost_usd` and `cost_usd` field names.
    ///
    /// # Returns
    ///
    /// An [`IterationCosts`] populated with cost, duration, and usage data
    /// from this result event.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph_core::stream::{ResultEvent, Usage};
    ///
    /// let event = ResultEvent {
    ///     subtype: Some("success".to_string()),
    ///     total_cost_usd: Some(0.15),
    ///     cost_usd: None,
    ///     duration_ms: Some(30000),
    ///     duration_api_ms: None,
    ///     usage: Some(Usage {
    ///         input_tokens: 500,
    ///         output_tokens: 1500,
    ///         cache_read_input_tokens: Some(10000),
    ///         cache_creation_input_tokens: None,
    ///     }),
    ///     session_id: None,
    ///     num_turns: None,
    ///     result: None,
    /// };
    ///
    /// let costs = event.extract_costs();
    /// assert_eq!(costs.cost_usd, Some(0.15));
    /// assert_eq!(costs.duration_ms, Some(30000));
    /// assert!(costs.usage.is_some());
    /// let usage = costs.usage.unwrap();
    /// assert_eq!(usage.input_tokens, 500);
    /// assert_eq!(usage.output_tokens, 1500);
    /// ```
    pub fn extract_costs(&self) -> IterationCosts {
        IterationCosts {
            cost_usd: self.get_cost_usd(),
            duration_ms: self.duration_ms,
            usage: self.usage.clone(),
        }
    }
}

/// Extract costs and usage from a slice of stream events.
///
/// Searches for the first result event and extracts cost/usage data from it.
/// Returns `None` if no result event is found.
///
/// # Arguments
///
/// * `events` - A slice of stream events to search
///
/// # Returns
///
/// `Some(IterationCosts)` if a result event was found, `None` otherwise.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     extract_costs_from_events, StreamEvent, ResultEvent, AssistantEvent,
///     AssistantMessage, ContentBlock, Usage,
/// };
///
/// let events = vec![
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: Some("msg_01".to_string()),
///             content: vec![ContentBlock::Text { text: "Hello!".to_string() }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
///     StreamEvent::Result(ResultEvent {
///         subtype: Some("success".to_string()),
///         total_cost_usd: Some(0.05),
///         cost_usd: None,
///         duration_ms: Some(5000),
///         duration_api_ms: None,
///         usage: Some(Usage {
///             input_tokens: 100,
///             output_tokens: 200,
///             cache_read_input_tokens: None,
///             cache_creation_input_tokens: None,
///         }),
///         session_id: None,
///         num_turns: None,
///         result: None,
///     }),
/// ];
///
/// let costs = extract_costs_from_events(&events);
/// assert!(costs.is_some());
/// let c = costs.unwrap();
/// assert_eq!(c.cost_usd, Some(0.05));
/// assert_eq!(c.duration_ms, Some(5000));
/// ```
pub fn extract_costs_from_events(events: &[StreamEvent]) -> Option<IterationCosts> {
    events.iter().find_map(|event| {
        if let StreamEvent::Result(res) = event {
            return Some(res.extract_costs());
        }
        None
    })
}

/// Extract costs and usage from a slice of stream events, returning default if not found.
///
/// Similar to [`extract_costs_from_events`], but returns a default (empty)
/// [`IterationCosts`] instead of `None` when no result event is found.
/// This is useful when you want to proceed with default values.
///
/// # Arguments
///
/// * `events` - A slice of stream events to search
///
/// # Returns
///
/// An [`IterationCosts`] populated from the result event, or a default empty struct.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     extract_costs_from_events_or_default, StreamEvent, AssistantEvent,
///     AssistantMessage, ContentBlock,
/// };
///
/// // No result event present - returns default
/// let events = vec![
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: None,
///             content: vec![ContentBlock::Text { text: "Hello!".to_string() }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// let costs = extract_costs_from_events_or_default(&events);
/// assert!(costs.is_empty());
/// assert_eq!(costs.cost_usd, None);
/// assert_eq!(costs.duration_ms, None);
/// assert!(costs.usage.is_none());
/// ```
pub fn extract_costs_from_events_or_default(events: &[StreamEvent]) -> IterationCosts {
    extract_costs_from_events(events).unwrap_or_default()
}

// Re-export chunk types for convenience when working with stream parsing
pub use crate::chunk::{parse_chunks, parse_chunks_with_heuristics, ChunkType, ParsedChunk};

/// Parse accumulated text from stream events into typed chunks.
///
/// Takes accumulated text from assistant events (obtained via [`accumulate_text`] or
/// [`TextAccumulator`]) and parses it into typed chunks: prose, code, and diff.
///
/// This is a convenience function that applies the chunk parsing logic from
/// [`crate::chunk::parse_chunks`] to text extracted from stream events.
///
/// # Arguments
///
/// * `events` - A slice of stream events to process
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     parse_chunks_from_events, StreamEvent, AssistantEvent, AssistantMessage,
///     ContentBlock, ChunkType,
/// };
///
/// let events = vec![
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: Some("msg_01".to_string()),
///             content: vec![ContentBlock::Text {
///                 text: "Here's the code:\n\n```rust\nfn main() {}\n```\n\nDone!".to_string(),
///             }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// let chunks = parse_chunks_from_events(&events);
/// assert_eq!(chunks.len(), 3);
/// assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
/// assert!(matches!(chunks[1].chunk_type, ChunkType::Code { .. }));
/// assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
/// ```
pub fn parse_chunks_from_events(events: &[StreamEvent]) -> Vec<ParsedChunk> {
    let text = accumulate_text(events.iter());
    parse_chunks(&text)
}

/// Parse accumulated text from stream events into typed chunks with diff heuristics.
///
/// Similar to [`parse_chunks_from_events`], but also applies heuristic detection for
/// unfenced diff content (e.g., raw `diff --git` output not wrapped in code fences).
///
/// Use this function when processing output that may contain unfenced diffs,
/// such as when the LLM outputs raw git diff results directly.
///
/// # Arguments
///
/// * `events` - A slice of stream events to process
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     parse_chunks_from_events_with_heuristics, StreamEvent, AssistantEvent,
///     AssistantMessage, ContentBlock, ChunkType,
/// };
///
/// let events = vec![
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: Some("msg_01".to_string()),
///             content: vec![ContentBlock::Text {
///                 text: "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new".to_string(),
///             }],
///             model: None,
///             stop_reason: None,
///         },
///     }),
/// ];
///
/// let chunks = parse_chunks_from_events_with_heuristics(&events);
/// assert_eq!(chunks.len(), 1);
/// assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
/// ```
pub fn parse_chunks_from_events_with_heuristics(events: &[StreamEvent]) -> Vec<ParsedChunk> {
    let text = accumulate_text(events.iter());
    parse_chunks_with_heuristics(&text)
}

/// Parse text directly into typed chunks.
///
/// This is a re-export of [`crate::chunk::parse_chunks`] for convenience when working
/// with stream parsing. See that function for detailed documentation.
///
/// # Arguments
///
/// * `text` - The text to parse (typically accumulated from assistant events)
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
pub fn parse_text_into_chunks(text: &str) -> Vec<ParsedChunk> {
    parse_chunks(text)
}

/// Parse text directly into typed chunks with diff heuristics.
///
/// This is a re-export of [`crate::chunk::parse_chunks_with_heuristics`] for convenience
/// when working with stream parsing. See that function for detailed documentation.
///
/// # Arguments
///
/// * `text` - The text to parse (typically accumulated from assistant events)
///
/// # Returns
///
/// An ordered list of [`ParsedChunk`] representing prose, code, and diff sections.
pub fn parse_text_into_chunks_with_heuristics(text: &str) -> Vec<ParsedChunk> {
    parse_chunks_with_heuristics(text)
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

    // Tests for text accumulation across streaming events (Story #15)

    #[test]
    fn test_text_accumulator_new() {
        let accumulator = TextAccumulator::new();
        assert_eq!(accumulator.get_text(), "");
        assert!(accumulator.completed_messages().is_empty());
    }

    #[test]
    fn test_text_accumulator_single_event() {
        let mut accumulator = TextAccumulator::new();

        let event = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Hello, world!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        let completed = accumulator.process_event(&event);
        assert!(completed.is_none()); // No message completed yet
        assert_eq!(accumulator.get_text(), "Hello, world!");
    }

    #[test]
    fn test_text_accumulator_multiple_events_same_message_id() {
        let mut accumulator = TextAccumulator::new();

        let event1 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Hello, ".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        let event2 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "world!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        accumulator.process_event(&event1);
        let completed = accumulator.process_event(&event2);

        assert!(completed.is_none()); // Same message ID, not completed yet
        assert_eq!(accumulator.get_text(), "Hello, world!");
    }

    #[test]
    fn test_text_accumulator_new_message_id_completes_previous() {
        let mut accumulator = TextAccumulator::new();

        let event1 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "First message.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        let event2 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_02".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Second message.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        accumulator.process_event(&event1);
        let completed = accumulator.process_event(&event2);

        // First message should be completed when we see msg_02
        assert!(completed.is_some());
        let msg = completed.unwrap();
        assert_eq!(msg.id, Some("msg_01".to_string()));
        assert_eq!(msg.text, "First message.");

        // Current buffer should have second message
        assert_eq!(accumulator.get_text(), "Second message.");
    }

    #[test]
    fn test_text_accumulator_finish() {
        let mut accumulator = TextAccumulator::new();

        let event = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Final message.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        accumulator.process_event(&event);
        let completed = accumulator.finish();

        assert!(completed.is_some());
        let msg = completed.unwrap();
        assert_eq!(msg.id, Some("msg_01".to_string()));
        assert_eq!(msg.text, "Final message.");

        // Buffer should be empty after finish
        assert_eq!(accumulator.get_text(), "");
    }

    #[test]
    fn test_text_accumulator_finish_empty() {
        let mut accumulator = TextAccumulator::new();
        let completed = accumulator.finish();
        assert!(completed.is_none());
    }

    #[test]
    fn test_text_accumulator_completed_messages() {
        let mut accumulator = TextAccumulator::new();

        let event1 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "First.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        let event2 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_02".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Second.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        accumulator.process_event(&event1);
        accumulator.process_event(&event2);
        accumulator.finish();

        let completed = accumulator.completed_messages();
        assert_eq!(completed.len(), 2);
        assert_eq!(completed[0].text, "First.");
        assert_eq!(completed[1].text, "Second.");
    }

    #[test]
    fn test_text_accumulator_get_all_text() {
        let mut accumulator = TextAccumulator::new();

        let event1 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "First. ".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        let event2 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_02".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Second.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        accumulator.process_event(&event1);
        accumulator.process_event(&event2);

        // Before finish: first message is completed, second is in buffer
        assert_eq!(accumulator.get_all_text(), "First. Second.");

        accumulator.finish();

        // After finish: both messages are completed
        assert_eq!(accumulator.get_all_text(), "First. Second.");
    }

    #[test]
    fn test_text_accumulator_ignores_non_assistant_events() {
        let mut accumulator = TextAccumulator::new();

        let system_event = StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("abc".to_string()),
            model: None,
            tools: vec![],
        });

        let user_event = StreamEvent::User(UserEvent {
            message: UserMessage {
                id: None,
                content: vec![ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_01".to_string()),
                    content: Some("result".to_string()),
                    is_error: false,
                }],
            },
        });

        let result_event = StreamEvent::Result(ResultEvent {
            subtype: None,
            total_cost_usd: Some(0.01),
            cost_usd: None,
            duration_ms: None,
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        });

        assert!(accumulator.process_event(&system_event).is_none());
        assert!(accumulator.process_event(&user_event).is_none());
        assert!(accumulator.process_event(&result_event).is_none());

        assert_eq!(accumulator.get_text(), "");
    }

    #[test]
    fn test_text_accumulator_reset() {
        let mut accumulator = TextAccumulator::new();

        let event = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Some text.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        accumulator.process_event(&event);
        accumulator.finish();

        assert!(!accumulator.completed_messages().is_empty());

        accumulator.reset();

        assert_eq!(accumulator.get_text(), "");
        assert!(accumulator.completed_messages().is_empty());
    }

    #[test]
    fn test_text_accumulator_with_none_message_ids() {
        let mut accumulator = TextAccumulator::new();

        // First event with no message ID
        let event1 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text {
                    text: "First.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        // Second event with no message ID (same as first)
        let event2 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text {
                    text: "Second.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        accumulator.process_event(&event1);
        let completed = accumulator.process_event(&event2);

        // Same message ID (None), so should combine
        assert!(completed.is_none());
        assert_eq!(accumulator.get_text(), "First.Second.");
    }

    #[test]
    fn test_text_accumulator_none_to_some_message_id() {
        let mut accumulator = TextAccumulator::new();

        let event1 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text {
                    text: "First.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        let event2 = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Second.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        });

        accumulator.process_event(&event1);
        let completed = accumulator.process_event(&event2);

        // ID changed from None to Some, should complete first message
        assert!(completed.is_some());
        let msg = completed.unwrap();
        assert_eq!(msg.id, None);
        assert_eq!(msg.text, "First.");
    }

    #[test]
    fn test_text_accumulator_tool_only_events() {
        let mut accumulator = TextAccumulator::new();

        let event = StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({"file_path": "/test.rs"}),
                }],
                model: None,
                stop_reason: None,
            },
        });

        accumulator.process_event(&event);

        // Tool-only events add empty string to buffer
        assert_eq!(accumulator.get_text(), "");
    }

    #[test]
    fn test_accumulate_text_function() {
        let events = [
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: None,
                model: None,
                tools: vec![],
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "Hello, ".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "world!".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
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

        let text = accumulate_text(events.iter());
        assert_eq!(text, "Hello, world!");
    }

    #[test]
    fn test_accumulate_text_multiple_messages() {
        let events = [
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "First message. ".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_02".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "Second message.".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
        ];

        let text = accumulate_text(events.iter());
        assert_eq!(text, "First message. Second message.");
    }

    #[test]
    fn test_accumulate_text_empty_events() {
        let events: Vec<StreamEvent> = vec![];
        let text = accumulate_text(events.iter());
        assert_eq!(text, "");
    }

    #[test]
    fn test_accumulate_text_matches_extract_text_from_events() {
        // This test verifies that accumulate_text produces the same result
        // as extract_text_from_events (matching plain-text mode output)
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: None,
                model: None,
                tools: vec![],
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![
                        ContentBlock::Text {
                            text: "Let me search. ".to_string(),
                        },
                        ContentBlock::ToolUse {
                            id: "toolu_01".to_string(),
                            name: "Glob".to_string(),
                            input: serde_json::json!({}),
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
                        content: Some("/test.rs".to_string()),
                        is_error: false,
                    }],
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_02".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "Found it!".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
        ];

        let accumulated = accumulate_text(events.iter());
        let extracted = extract_text_from_events(&events);

        assert_eq!(accumulated, extracted);
    }

    #[test]
    fn test_text_accumulator_realistic_streaming_scenario() {
        // Simulates a realistic Claude streaming scenario where text arrives
        // in chunks across multiple events with the same message ID
        let mut accumulator = TextAccumulator::new();

        let events = vec![
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "I'll ".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "help you ".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "implement ".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "this feature.".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
        ];

        for event in &events {
            accumulator.process_event(event);
        }

        assert_eq!(
            accumulator.get_text(),
            "I'll help you implement this feature."
        );

        // Finish and verify
        accumulator.finish();
        assert_eq!(accumulator.completed_messages().len(), 1);
        assert_eq!(
            accumulator.get_all_text(),
            "I'll help you implement this feature."
        );
    }

    // Tests for chunk parsing from events (Story #16)

    #[test]
    fn test_parse_chunks_from_events_prose_only() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Hello, world!\nThis is prose.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let chunks = parse_chunks_from_events(&events);
        assert_eq!(chunks.len(), 1);
        assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
        assert_eq!(chunks[0].content, "Hello, world!\nThis is prose.");
    }

    #[test]
    fn test_parse_chunks_from_events_with_code_block() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Here's the code:\n\n```rust\nfn main() {}\n```\n\nDone!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let chunks = parse_chunks_from_events(&events);
        assert_eq!(chunks.len(), 3);

        // First chunk: prose
        assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
        assert!(chunks[0].content.contains("Here's the code:"));

        // Second chunk: code
        match &chunks[1].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("rust"));
            }
            _ => panic!("Expected code chunk"),
        }
        assert_eq!(chunks[1].content, "fn main() {}");

        // Third chunk: prose
        assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
        assert!(chunks[2].content.contains("Done!"));
    }

    #[test]
    fn test_parse_chunks_from_events_with_diff_block() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Changes:\n\n```diff\n-old\n+new\n```\n\nApplied.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let chunks = parse_chunks_from_events(&events);
        assert_eq!(chunks.len(), 3);

        // First chunk: prose
        assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));

        // Second chunk: diff
        assert!(matches!(chunks[1].chunk_type, ChunkType::Diff));
        assert_eq!(chunks[1].content, "-old\n+new");

        // Third chunk: prose
        assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
    }

    #[test]
    fn test_parse_chunks_from_events_multiple_code_blocks() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "```python\nprint('hello')\n```\n\nand\n\n```javascript\nconsole.log('hi')\n```".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let chunks = parse_chunks_from_events(&events);
        assert_eq!(chunks.len(), 3);

        // First code block
        match &chunks[0].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("python"));
            }
            _ => panic!("Expected code chunk"),
        }

        // Prose between
        assert!(matches!(chunks[1].chunk_type, ChunkType::Prose));

        // Second code block
        match &chunks[2].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("javascript"));
            }
            _ => panic!("Expected code chunk"),
        }
    }

    #[test]
    fn test_parse_chunks_from_events_empty() {
        let events: Vec<StreamEvent> = vec![];
        let chunks = parse_chunks_from_events(&events);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_parse_chunks_from_events_non_assistant_events() {
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
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

        let chunks = parse_chunks_from_events(&events);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_parse_chunks_from_events_multiple_assistant_events() {
        let events = vec![
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "First part\n\n```rust\nfn a() {}\n```".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "\n\nSecond part\n\n```python\ndef b():\n    pass\n```".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
        ];

        let chunks = parse_chunks_from_events(&events);
        // Should have: prose, rust code, prose, python code
        assert_eq!(chunks.len(), 4);

        assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
        match &chunks[1].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("rust"));
            }
            _ => panic!("Expected rust code chunk"),
        }
        assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
        match &chunks[3].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("python"));
            }
            _ => panic!("Expected python code chunk"),
        }
    }

    #[test]
    fn test_parse_chunks_from_events_with_heuristics_unfenced_diff() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let chunks = parse_chunks_from_events_with_heuristics(&events);
        assert_eq!(chunks.len(), 1);
        assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
    }

    #[test]
    fn test_parse_chunks_from_events_with_heuristics_fenced_code_preserved() {
        // Fenced code blocks should still be detected correctly
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "```rust\nfn main() {}\n```".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let chunks = parse_chunks_from_events_with_heuristics(&events);
        assert_eq!(chunks.len(), 1);
        match &chunks[0].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("rust"));
            }
            _ => panic!("Expected code chunk"),
        }
    }

    #[test]
    fn test_parse_text_into_chunks_code_block() {
        let text = "Here's code:\n\n```rust\nfn main() {}\n```\n\nDone!";
        let chunks = parse_text_into_chunks(text);

        assert_eq!(chunks.len(), 3);
        assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
        match &chunks[1].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("rust"));
            }
            _ => panic!("Expected code chunk"),
        }
        assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
    }

    #[test]
    fn test_parse_text_into_chunks_with_heuristics() {
        let text = "diff --git a/f.rs b/f.rs\n-old\n+new";
        let chunks = parse_text_into_chunks_with_heuristics(text);

        assert_eq!(chunks.len(), 1);
        assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
    }

    #[test]
    fn test_parse_chunks_preserves_language_metadata() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "```typescript\nconst x: number = 1;\n```".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let chunks = parse_chunks_from_events(&events);
        assert_eq!(chunks.len(), 1);
        match &chunks[0].chunk_type {
            ChunkType::Code { language } => {
                assert_eq!(language.as_deref(), Some("typescript"));
            }
            _ => panic!("Expected code chunk with language"),
        }
    }

    #[test]
    fn test_parse_chunks_prose_between_code_blocks_preserved() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "First\n\n```rust\nfn a() {}\n```\n\nMiddle text here\n\n```python\ndef b(): pass\n```\n\nLast".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let chunks = parse_chunks_from_events(&events);
        assert_eq!(chunks.len(), 5);

        // Prose - code - prose - code - prose
        assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
        assert!(matches!(chunks[1].chunk_type, ChunkType::Code { .. }));
        assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
        assert!(chunks[2].content.contains("Middle text here"));
        assert!(matches!(chunks[3].chunk_type, ChunkType::Code { .. }));
        assert!(matches!(chunks[4].chunk_type, ChunkType::Prose));
    }

    #[test]
    fn test_parse_chunks_matches_plain_text_parsing() {
        // Verify that parsing from events produces the same result as parsing plain text directly
        let text = "Intro\n\n```rust\nfn main() {\n    println!(\"Hello\");\n}\n```\n\nOutro";

        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: text.to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let chunks_from_events = parse_chunks_from_events(&events);
        let chunks_from_text = parse_text_into_chunks(text);

        assert_eq!(chunks_from_events.len(), chunks_from_text.len());
        for (a, b) in chunks_from_events.iter().zip(chunks_from_text.iter()) {
            assert_eq!(a.chunk_type, b.chunk_type);
            assert_eq!(a.content, b.content);
        }
    }

    // ==========================================================================
    // IterationMetadata extraction tests (Story #17)
    // ==========================================================================

    #[test]
    fn test_iteration_metadata_new() {
        let metadata = IterationMetadata::new();
        assert!(metadata.is_empty());
        assert_eq!(metadata.session_id, None);
        assert_eq!(metadata.model, None);
        assert!(metadata.tools.is_empty());
    }

    #[test]
    fn test_iteration_metadata_is_empty() {
        let empty = IterationMetadata::default();
        assert!(empty.is_empty());

        let with_session_id = IterationMetadata {
            session_id: Some("abc".to_string()),
            ..Default::default()
        };
        assert!(!with_session_id.is_empty());

        let with_model = IterationMetadata {
            model: Some("claude".to_string()),
            ..Default::default()
        };
        assert!(!with_model.is_empty());

        let with_tools = IterationMetadata {
            tools: vec![Tool {
                name: "Read".to_string(),
                description: None,
            }],
            ..Default::default()
        };
        assert!(!with_tools.is_empty());
    }

    #[test]
    fn test_system_event_is_init() {
        let init_event = SystemEvent {
            subtype: Some("init".to_string()),
            session_id: None,
            model: None,
            tools: vec![],
        };
        assert!(init_event.is_init());

        let other_event = SystemEvent {
            subtype: Some("other".to_string()),
            session_id: None,
            model: None,
            tools: vec![],
        };
        assert!(!other_event.is_init());

        let no_subtype = SystemEvent {
            subtype: None,
            session_id: None,
            model: None,
            tools: vec![],
        };
        assert!(!no_subtype.is_init());
    }

    #[test]
    fn test_system_event_extract_metadata() {
        let event = SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("session-123".to_string()),
            model: Some("claude-opus-4-5-20251101".to_string()),
            tools: vec![
                Tool {
                    name: "Read".to_string(),
                    description: Some("Read files".to_string()),
                },
                Tool {
                    name: "Edit".to_string(),
                    description: None,
                },
            ],
        };

        let metadata = event.extract_metadata();
        assert_eq!(metadata.session_id, Some("session-123".to_string()));
        assert_eq!(metadata.model, Some("claude-opus-4-5-20251101".to_string()));
        assert_eq!(metadata.tools.len(), 2);
        assert_eq!(metadata.tools[0].name, "Read");
        assert_eq!(
            metadata.tools[0].description,
            Some("Read files".to_string())
        );
        assert_eq!(metadata.tools[1].name, "Edit");
        assert_eq!(metadata.tools[1].description, None);
    }

    #[test]
    fn test_system_event_extract_metadata_missing_fields() {
        let event = SystemEvent {
            subtype: Some("init".to_string()),
            session_id: None,
            model: None,
            tools: vec![],
        };

        let metadata = event.extract_metadata();
        assert_eq!(metadata.session_id, None);
        assert_eq!(metadata.model, None);
        assert!(metadata.tools.is_empty());
        assert!(metadata.is_empty());
    }

    #[test]
    fn test_extract_metadata_from_events_init_present() {
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: Some("f5b6aaac-4316-454a".to_string()),
                model: Some("claude-opus-4-5-20251101".to_string()),
                tools: vec![
                    Tool {
                        name: "Glob".to_string(),
                        description: Some("Find files".to_string()),
                    },
                    Tool {
                        name: "Read".to_string(),
                        description: None,
                    },
                ],
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "Hello!".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
        ];

        let metadata = extract_metadata_from_events(&events);
        assert!(metadata.is_some());
        let meta = metadata.unwrap();
        assert_eq!(meta.session_id, Some("f5b6aaac-4316-454a".to_string()));
        assert_eq!(meta.model, Some("claude-opus-4-5-20251101".to_string()));
        assert_eq!(meta.tools.len(), 2);
        assert_eq!(meta.tools[0].name, "Glob");
        assert_eq!(meta.tools[1].name, "Read");
    }

    #[test]
    fn test_extract_metadata_from_events_no_system_event() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Hello!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let metadata = extract_metadata_from_events(&events);
        assert!(metadata.is_none());
    }

    #[test]
    fn test_extract_metadata_from_events_non_init_system_event() {
        let events = vec![StreamEvent::System(SystemEvent {
            subtype: Some("other".to_string()),
            session_id: Some("abc".to_string()),
            model: Some("claude".to_string()),
            tools: vec![],
        })];

        let metadata = extract_metadata_from_events(&events);
        assert!(metadata.is_none());
    }

    #[test]
    fn test_extract_metadata_from_events_empty_events() {
        let events: Vec<StreamEvent> = vec![];
        let metadata = extract_metadata_from_events(&events);
        assert!(metadata.is_none());
    }

    #[test]
    fn test_extract_metadata_from_events_first_init_wins() {
        // If there are multiple init events, the first one should be returned
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: Some("first-session".to_string()),
                model: Some("model-1".to_string()),
                tools: vec![],
            }),
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: Some("second-session".to_string()),
                model: Some("model-2".to_string()),
                tools: vec![],
            }),
        ];

        let metadata = extract_metadata_from_events(&events);
        assert!(metadata.is_some());
        let meta = metadata.unwrap();
        assert_eq!(meta.session_id, Some("first-session".to_string()));
        assert_eq!(meta.model, Some("model-1".to_string()));
    }

    #[test]
    fn test_extract_metadata_from_events_or_default_with_init() {
        let events = vec![StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("session-456".to_string()),
            model: Some("claude-sonnet".to_string()),
            tools: vec![Tool {
                name: "Write".to_string(),
                description: None,
            }],
        })];

        let metadata = extract_metadata_from_events_or_default(&events);
        assert!(!metadata.is_empty());
        assert_eq!(metadata.session_id, Some("session-456".to_string()));
        assert_eq!(metadata.model, Some("claude-sonnet".to_string()));
        assert_eq!(metadata.tools.len(), 1);
    }

    #[test]
    fn test_extract_metadata_from_events_or_default_no_init() {
        let events: Vec<StreamEvent> = vec![];
        let metadata = extract_metadata_from_events_or_default(&events);
        assert!(metadata.is_empty());
        assert_eq!(metadata.session_id, None);
        assert_eq!(metadata.model, None);
        assert!(metadata.tools.is_empty());
    }

    #[test]
    fn test_iteration_metadata_serialization() {
        let metadata = IterationMetadata {
            session_id: Some("abc-123".to_string()),
            model: Some("claude-opus-4-5".to_string()),
            tools: vec![
                Tool {
                    name: "Read".to_string(),
                    description: Some("Read files".to_string()),
                },
                Tool {
                    name: "Edit".to_string(),
                    description: None,
                },
            ],
        };

        let json = serde_json::to_string(&metadata).unwrap();
        let roundtrip: IterationMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(metadata, roundtrip);
    }

    #[test]
    fn test_iteration_metadata_serialization_empty_fields_skipped() {
        let metadata = IterationMetadata {
            session_id: Some("abc".to_string()),
            model: None,
            tools: vec![],
        };

        let json = serde_json::to_string(&metadata).unwrap();
        // Empty model and tools should be skipped
        assert!(!json.contains("\"model\""));
        assert!(!json.contains("\"tools\""));
        // But session_id should be present
        assert!(json.contains("\"session_id\""));
    }

    #[test]
    fn test_extract_metadata_init_at_end() {
        // Init event at the end of the list (should still be found)
        let events = vec![
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "Hello!".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Result(ResultEvent {
                subtype: None,
                total_cost_usd: Some(0.01),
                cost_usd: None,
                duration_ms: None,
                duration_api_ms: None,
                usage: None,
                session_id: None,
                num_turns: None,
                result: None,
            }),
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: Some("late-init".to_string()),
                model: Some("claude-late".to_string()),
                tools: vec![],
            }),
        ];

        let metadata = extract_metadata_from_events(&events);
        assert!(metadata.is_some());
        let meta = metadata.unwrap();
        assert_eq!(meta.session_id, Some("late-init".to_string()));
    }

    #[test]
    fn test_extract_metadata_partial_fields() {
        // Only some fields present - others should be None/empty
        let events = vec![StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("session-only".to_string()),
            model: None,
            tools: vec![],
        })];

        let metadata = extract_metadata_from_events(&events);
        assert!(metadata.is_some());
        let meta = metadata.unwrap();
        assert_eq!(meta.session_id, Some("session-only".to_string()));
        assert_eq!(meta.model, None);
        assert!(meta.tools.is_empty());
        assert!(!meta.is_empty()); // Still not empty since session_id is set
    }

    // =========================================================================
    // IterationCosts Tests
    // =========================================================================

    #[test]
    fn test_iteration_costs_new_is_empty() {
        let costs = IterationCosts::new();
        assert!(costs.is_empty());
        assert_eq!(costs.cost_usd, None);
        assert_eq!(costs.duration_ms, None);
        assert!(costs.usage.is_none());
    }

    #[test]
    fn test_iteration_costs_is_empty_with_cost() {
        let costs = IterationCosts {
            cost_usd: Some(0.05),
            duration_ms: None,
            usage: None,
        };
        assert!(!costs.is_empty());
    }

    #[test]
    fn test_iteration_costs_is_empty_with_duration() {
        let costs = IterationCosts {
            cost_usd: None,
            duration_ms: Some(5000),
            usage: None,
        };
        assert!(!costs.is_empty());
    }

    #[test]
    fn test_iteration_costs_is_empty_with_usage() {
        let costs = IterationCosts {
            cost_usd: None,
            duration_ms: None,
            usage: Some(Usage::default()),
        };
        assert!(!costs.is_empty());
    }

    #[test]
    fn test_result_event_extract_costs_full() {
        let event = ResultEvent {
            subtype: Some("success".to_string()),
            total_cost_usd: Some(0.226354),
            cost_usd: None,
            duration_ms: Some(40966),
            duration_api_ms: Some(35000),
            usage: Some(Usage {
                input_tokens: 712,
                output_tokens: 2971,
                cache_read_input_tokens: Some(107476),
                cache_creation_input_tokens: Some(12504),
            }),
            session_id: Some("session-123".to_string()),
            num_turns: Some(3),
            result: None,
        };

        let costs = event.extract_costs();
        assert_eq!(costs.cost_usd, Some(0.226354));
        assert_eq!(costs.duration_ms, Some(40966));
        assert!(costs.usage.is_some());
        let usage = costs.usage.unwrap();
        assert_eq!(usage.input_tokens, 712);
        assert_eq!(usage.output_tokens, 2971);
        assert_eq!(usage.cache_read_input_tokens, Some(107476));
        assert_eq!(usage.cache_creation_input_tokens, Some(12504));
    }

    #[test]
    fn test_result_event_extract_costs_uses_alternative_cost_field() {
        // When total_cost_usd is None but cost_usd is set
        let event = ResultEvent {
            subtype: None,
            total_cost_usd: None,
            cost_usd: Some(0.123),
            duration_ms: Some(1000),
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        };

        let costs = event.extract_costs();
        assert_eq!(costs.cost_usd, Some(0.123));
        assert_eq!(costs.duration_ms, Some(1000));
        assert!(costs.usage.is_none());
    }

    #[test]
    fn test_result_event_extract_costs_prefers_total_cost_usd() {
        // When both fields are set, total_cost_usd takes precedence
        let event = ResultEvent {
            subtype: None,
            total_cost_usd: Some(0.50),
            cost_usd: Some(0.25),
            duration_ms: None,
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        };

        let costs = event.extract_costs();
        assert_eq!(costs.cost_usd, Some(0.50)); // total_cost_usd wins
    }

    #[test]
    fn test_result_event_extract_costs_empty() {
        let event = ResultEvent {
            subtype: None,
            total_cost_usd: None,
            cost_usd: None,
            duration_ms: None,
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        };

        let costs = event.extract_costs();
        assert!(costs.is_empty());
    }

    #[test]
    fn test_extract_costs_from_events_with_result() {
        let events = vec![
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "Hello!".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
            StreamEvent::Result(ResultEvent {
                subtype: Some("success".to_string()),
                total_cost_usd: Some(0.05),
                cost_usd: None,
                duration_ms: Some(5000),
                duration_api_ms: None,
                usage: Some(Usage {
                    input_tokens: 100,
                    output_tokens: 200,
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                }),
                session_id: None,
                num_turns: None,
                result: None,
            }),
        ];

        let costs = extract_costs_from_events(&events);
        assert!(costs.is_some());
        let c = costs.unwrap();
        assert_eq!(c.cost_usd, Some(0.05));
        assert_eq!(c.duration_ms, Some(5000));
        assert!(c.usage.is_some());
    }

    #[test]
    fn test_extract_costs_from_events_no_result() {
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: Some("session-123".to_string()),
                model: Some("claude-opus-4-5".to_string()),
                tools: vec![],
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "Hello!".to_string(),
                    }],
                    model: None,
                    stop_reason: None,
                },
            }),
        ];

        let costs = extract_costs_from_events(&events);
        assert!(costs.is_none());
    }

    #[test]
    fn test_extract_costs_from_events_empty_slice() {
        let events: Vec<StreamEvent> = vec![];
        let costs = extract_costs_from_events(&events);
        assert!(costs.is_none());
    }

    #[test]
    fn test_extract_costs_from_events_multiple_results_takes_first() {
        let events = vec![
            StreamEvent::Result(ResultEvent {
                subtype: None,
                total_cost_usd: Some(0.10),
                cost_usd: None,
                duration_ms: Some(1000),
                duration_api_ms: None,
                usage: None,
                session_id: None,
                num_turns: None,
                result: None,
            }),
            StreamEvent::Result(ResultEvent {
                subtype: None,
                total_cost_usd: Some(0.20),
                cost_usd: None,
                duration_ms: Some(2000),
                duration_api_ms: None,
                usage: None,
                session_id: None,
                num_turns: None,
                result: None,
            }),
        ];

        let costs = extract_costs_from_events(&events);
        assert!(costs.is_some());
        let c = costs.unwrap();
        assert_eq!(c.cost_usd, Some(0.10)); // First result wins
        assert_eq!(c.duration_ms, Some(1000));
    }

    #[test]
    fn test_extract_costs_from_events_or_default_with_result() {
        let events = vec![StreamEvent::Result(ResultEvent {
            subtype: None,
            total_cost_usd: Some(0.15),
            cost_usd: None,
            duration_ms: Some(3000),
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        })];

        let costs = extract_costs_from_events_or_default(&events);
        assert!(!costs.is_empty());
        assert_eq!(costs.cost_usd, Some(0.15));
        assert_eq!(costs.duration_ms, Some(3000));
    }

    #[test]
    fn test_extract_costs_from_events_or_default_no_result() {
        let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text {
                    text: "Hello!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

        let costs = extract_costs_from_events_or_default(&events);
        assert!(costs.is_empty());
        assert_eq!(costs.cost_usd, None);
        assert_eq!(costs.duration_ms, None);
        assert!(costs.usage.is_none());
    }

    #[test]
    fn test_iteration_costs_serialization_round_trip() {
        let costs = IterationCosts {
            cost_usd: Some(0.226354),
            duration_ms: Some(40966),
            usage: Some(Usage {
                input_tokens: 712,
                output_tokens: 2971,
                cache_read_input_tokens: Some(107476),
                cache_creation_input_tokens: Some(12504),
            }),
        };

        let json = serde_json::to_string(&costs).unwrap();
        let parsed: IterationCosts = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, costs);
    }

    #[test]
    fn test_iteration_costs_empty_fields_skip_serialization() {
        let costs = IterationCosts {
            cost_usd: Some(0.05),
            duration_ms: None,
            usage: None,
        };

        let json = serde_json::to_string(&costs).unwrap();
        // Should not contain duration_ms or usage fields
        assert!(!json.contains("duration_ms"));
        assert!(!json.contains("usage"));
        assert!(json.contains("cost_usd"));
    }

    #[test]
    fn test_extract_costs_result_at_end() {
        // Result event typically comes at the end of the stream
        let events = vec![
            StreamEvent::System(SystemEvent {
                subtype: Some("init".to_string()),
                session_id: Some("session-123".to_string()),
                model: Some("claude-opus-4-5".to_string()),
                tools: vec![],
            }),
            StreamEvent::Assistant(AssistantEvent {
                message: AssistantMessage {
                    id: Some("msg_01".to_string()),
                    content: vec![ContentBlock::Text {
                        text: "Implementing feature...".to_string(),
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
                        content: Some("File updated".to_string()),
                        is_error: false,
                    }],
                },
            }),
            StreamEvent::Result(ResultEvent {
                subtype: Some("success".to_string()),
                total_cost_usd: Some(0.30),
                cost_usd: None,
                duration_ms: Some(60000),
                duration_api_ms: None,
                usage: Some(Usage {
                    input_tokens: 1000,
                    output_tokens: 5000,
                    cache_read_input_tokens: Some(50000),
                    cache_creation_input_tokens: None,
                }),
                session_id: None,
                num_turns: Some(5),
                result: None,
            }),
        ];

        let costs = extract_costs_from_events(&events);
        assert!(costs.is_some());
        let c = costs.unwrap();
        assert_eq!(c.cost_usd, Some(0.30));
        assert_eq!(c.duration_ms, Some(60000));
        let usage = c.usage.unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.output_tokens, 5000);
        assert_eq!(usage.cache_read_input_tokens, Some(50000));
        assert_eq!(usage.cache_creation_input_tokens, None);
    }
}
