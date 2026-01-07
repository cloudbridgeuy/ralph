//! Text and tool extraction from stream events.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::events::{AssistantEvent, AssistantMessage, ContentBlock, StreamEvent};

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
