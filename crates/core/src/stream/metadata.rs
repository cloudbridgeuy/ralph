//! Metadata extraction from system events.

use serde::{Deserialize, Serialize};

use super::events::{StreamEvent, SystemEvent, Tool};

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
