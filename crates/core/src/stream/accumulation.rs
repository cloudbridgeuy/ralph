//! Text accumulation utilities for streaming events.

use super::events::{AssistantEvent, StreamEvent};

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

// Re-export AssistantEvent methods that use ToolInvocation
impl AssistantEvent {
    // Note: extract_text and extract_tool_invocations are implemented in extraction.rs
}
