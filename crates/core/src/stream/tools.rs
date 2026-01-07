//! Tool call and result correlation.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::events::{StreamEvent, ToolResult};
use super::extraction::{extract_tool_invocations_from_events, ToolInvocation};

/// A complete tool interaction: the tool call paired with its result.
///
/// This struct correlates a tool invocation from an assistant event with
/// its corresponding result from a user event. The correlation is done
/// using the `tool_use_id` field that appears in both the `ToolUse` content
/// block and the `ToolResult` content.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{correlate_tool_interactions, StreamEvent, AssistantEvent, AssistantMessage, ContentBlock, UserEvent, UserMessage, ToolResult};
/// use serde_json::json;
///
/// let events = vec![
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: Some("msg_01".to_string()),
///             content: vec![ContentBlock::ToolUse {
///                 id: "toolu_01".to_string(),
///                 name: "Read".to_string(),
///                 input: json!({"file_path": "/src/main.rs"}),
///             }],
///             model: None,
///             stop_reason: Some("tool_use".to_string()),
///         },
///     }),
///     StreamEvent::User(UserEvent {
///         message: UserMessage {
///             id: Some("user_msg_01".to_string()),
///             content: vec![ToolResult {
///                 result_type: Some("tool_result".to_string()),
///                 tool_use_id: Some("toolu_01".to_string()),
///                 content: Some("fn main() {}".to_string()),
///                 is_error: false,
///             }],
///         },
///     }),
/// ];
///
/// let interactions = correlate_tool_interactions(&events);
/// assert_eq!(interactions.len(), 1);
/// assert_eq!(interactions[0].name, "Read");
/// assert_eq!(interactions[0].result, Some("fn main() {}".to_string()));
/// assert!(!interactions[0].is_error);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInteraction {
    /// Unique identifier for this tool use (from the assistant's ToolUse block).
    pub id: String,

    /// The name of the tool that was invoked (e.g., "Read", "Edit", "Glob").
    pub name: String,

    /// The input arguments to the tool as a JSON object.
    pub input: Value,

    /// The result content from the tool execution, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,

    /// Whether the tool execution resulted in an error.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolInteraction {
    /// Create a new ToolInteraction from an invocation (without result).
    ///
    /// This is used when a tool call has no corresponding result
    /// (e.g., the stream was interrupted or the tool failed silently).
    pub fn from_invocation(invocation: &ToolInvocation) -> Self {
        Self {
            id: invocation.id.clone(),
            name: invocation.name.clone(),
            input: invocation.input.clone(),
            result: None,
            is_error: false,
        }
    }

    /// Create a new ToolInteraction from an invocation and result.
    pub fn from_invocation_and_result(invocation: &ToolInvocation, result: &ToolResult) -> Self {
        Self {
            id: invocation.id.clone(),
            name: invocation.name.clone(),
            input: invocation.input.clone(),
            result: result.content.clone(),
            is_error: result.is_error,
        }
    }
}

/// Correlates tool calls from assistant events with their results from user events.
///
/// This function processes a sequence of stream events and pairs each tool
/// invocation with its corresponding result. Tool calls and results are
/// correlated using the `tool_use_id` field.
///
/// # Arguments
///
/// * `events` - A slice of stream events to process
///
/// # Returns
///
/// A `Vec<ToolInteraction>` containing all tool calls with their results.
/// Tool calls without results will have `result: None`.
/// The order matches the order of tool calls in the stream.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     correlate_tool_interactions, StreamEvent, AssistantEvent, AssistantMessage,
///     ContentBlock, UserEvent, UserMessage, ToolResult,
/// };
/// use serde_json::json;
///
/// let events = vec![
///     StreamEvent::Assistant(AssistantEvent {
///         message: AssistantMessage {
///             id: Some("msg_01".to_string()),
///             content: vec![
///                 ContentBlock::Text { text: "Let me read the file.".to_string() },
///                 ContentBlock::ToolUse {
///                     id: "toolu_01".to_string(),
///                     name: "Read".to_string(),
///                     input: json!({"file_path": "/src/main.rs"}),
///                 },
///             ],
///             model: None,
///             stop_reason: Some("tool_use".to_string()),
///         },
///     }),
///     StreamEvent::User(UserEvent {
///         message: UserMessage {
///             id: Some("user_msg_01".to_string()),
///             content: vec![ToolResult {
///                 result_type: Some("tool_result".to_string()),
///                 tool_use_id: Some("toolu_01".to_string()),
///                 content: Some("fn main() {\n    println!(\"Hello\");\n}".to_string()),
///                 is_error: false,
///             }],
///         },
///     }),
/// ];
///
/// let interactions = correlate_tool_interactions(&events);
/// assert_eq!(interactions.len(), 1);
/// assert_eq!(interactions[0].id, "toolu_01");
/// assert_eq!(interactions[0].name, "Read");
/// assert!(interactions[0].result.is_some());
/// assert!(!interactions[0].is_error);
/// ```
pub fn correlate_tool_interactions(events: &[StreamEvent]) -> Vec<ToolInteraction> {
    // Collect all tool results keyed by tool_use_id
    let mut results_by_id: HashMap<String, &ToolResult> = HashMap::new();
    for event in events {
        if let StreamEvent::User(user) = event {
            for result in &user.message.content {
                if let Some(ref tool_use_id) = result.tool_use_id {
                    results_by_id.insert(tool_use_id.clone(), result);
                }
            }
        }
    }

    // Extract all tool invocations and pair with results
    let invocations = extract_tool_invocations_from_events(events);
    invocations
        .iter()
        .map(|invocation| {
            if let Some(result) = results_by_id.get(&invocation.id) {
                ToolInteraction::from_invocation_and_result(invocation, result)
            } else {
                ToolInteraction::from_invocation(invocation)
            }
        })
        .collect()
}

/// A stateful correlator for tool calls and results during streaming.
///
/// This is useful for streaming scenarios where you want to process tool
/// interactions as they complete rather than waiting for the entire stream.
///
/// # Example
///
/// ```
/// use ralph_core::stream::{
///     ToolCorrelator, StreamEvent, AssistantEvent, AssistantMessage,
///     ContentBlock, UserEvent, UserMessage, ToolResult,
/// };
/// use serde_json::json;
///
/// let mut correlator = ToolCorrelator::new();
///
/// // Process assistant event with tool call
/// let assistant_event = StreamEvent::Assistant(AssistantEvent {
///     message: AssistantMessage {
///         id: Some("msg_01".to_string()),
///         content: vec![ContentBlock::ToolUse {
///             id: "toolu_01".to_string(),
///             name: "Glob".to_string(),
///             input: json!({"pattern": "*.rs"}),
///         }],
///         model: None,
///         stop_reason: Some("tool_use".to_string()),
///     },
/// });
/// let completed = correlator.process_event(&assistant_event);
/// assert!(completed.is_empty()); // No results yet
/// assert_eq!(correlator.pending_count(), 1);
///
/// // Process user event with tool result
/// let user_event = StreamEvent::User(UserEvent {
///     message: UserMessage {
///         id: Some("user_01".to_string()),
///         content: vec![ToolResult {
///             result_type: Some("tool_result".to_string()),
///             tool_use_id: Some("toolu_01".to_string()),
///             content: Some("src/main.rs\nsrc/lib.rs".to_string()),
///             is_error: false,
///         }],
///     },
/// });
/// let completed = correlator.process_event(&user_event);
/// assert_eq!(completed.len(), 1);
/// assert_eq!(completed[0].name, "Glob");
/// assert_eq!(correlator.pending_count(), 0);
/// ```
#[derive(Debug, Clone, Default)]
pub struct ToolCorrelator {
    /// Pending tool invocations waiting for results, keyed by tool_use_id.
    pending: HashMap<String, ToolInvocation>,
    /// All completed tool interactions (in order of completion).
    completed: Vec<ToolInteraction>,
    /// Order of tool invocations (to preserve order when finalizing).
    invocation_order: Vec<String>,
}

impl ToolCorrelator {
    /// Create a new empty tool correlator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a stream event, tracking tool calls and correlating with results.
    ///
    /// - Assistant events: Extracts tool invocations and adds them to pending.
    /// - User events: Matches tool results with pending invocations.
    /// - Other events are ignored.
    ///
    /// # Returns
    ///
    /// A `Vec<ToolInteraction>` containing any interactions that were completed
    /// by this event. This will be non-empty when a user event contains results
    /// for pending tool calls.
    pub fn process_event(&mut self, event: &StreamEvent) -> Vec<ToolInteraction> {
        match event {
            StreamEvent::Assistant(assistant) => {
                let invocations = assistant.extract_tool_invocations();
                for invocation in invocations {
                    self.invocation_order.push(invocation.id.clone());
                    self.pending.insert(invocation.id.clone(), invocation);
                }
                Vec::new()
            }
            StreamEvent::User(user) => {
                let mut newly_completed = Vec::new();
                for result in &user.message.content {
                    if let Some(ref tool_use_id) = result.tool_use_id {
                        if let Some(invocation) = self.pending.remove(tool_use_id) {
                            let interaction =
                                ToolInteraction::from_invocation_and_result(&invocation, result);
                            newly_completed.push(interaction.clone());
                            self.completed.push(interaction);
                        }
                    }
                }
                newly_completed
            }
            _ => Vec::new(),
        }
    }

    /// Get the number of pending (unresolved) tool calls.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Get all completed tool interactions so far.
    pub fn completed_interactions(&self) -> &[ToolInteraction] {
        &self.completed
    }

    /// Finalize the correlator and return all tool interactions.
    ///
    /// Tool calls that never received results will have `result: None`.
    /// The returned interactions are in the order the tool calls were made.
    pub fn finish(mut self) -> Vec<ToolInteraction> {
        // Process any remaining pending invocations (never got results)
        for id in &self.invocation_order {
            if let Some(invocation) = self.pending.remove(id) {
                self.completed
                    .push(ToolInteraction::from_invocation(&invocation));
            }
        }
        self.completed
    }

    /// Reset the correlator to its initial state.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.completed.clear();
        self.invocation_order.clear();
    }
}
