//! Cost and usage extraction from result events.

use serde::{Deserialize, Serialize};

use super::events::{ResultEvent, StreamEvent, Usage};

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
