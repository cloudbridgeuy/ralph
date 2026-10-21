//! Line parsing utilities for stream-JSON output.

use super::events::StreamEvent;

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

fn first_json_object(line: &str) -> Option<serde_json::Value> {
    let start = line.find('{')?;
    serde_json::Deserializer::from_str(&line[start..])
        .into_iter::<serde_json::Value>()
        .next()?
        .ok()
}

/// Returns `Some(is_error)` when the line's top-level `type` is `result`, and
/// `None` for any other line. A `type` nested inside the event (for example a
/// tool input) does not count. The first JSON object from the first `{` is
/// read, so surrounding whitespace, a prefix, or trailing text do not hide a
/// result.
pub fn result_is_error(line: &str) -> Option<bool> {
    let value = first_json_object(line)?;
    if value.get("type")?.as_str()? != "result" {
        return None;
    }
    Some(
        value
            .get("is_error")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    )
}

#[cfg(test)]
mod result_is_error_tests {
    use super::result_is_error;

    #[test]
    fn plain_result_is_not_error() {
        assert_eq!(
            result_is_error(r#"{"type":"result","subtype":"success","is_error":false}"#),
            Some(false)
        );
    }

    #[test]
    fn missing_is_error_is_false() {
        assert_eq!(result_is_error(r#"{"type":"result"}"#), Some(false));
    }

    #[test]
    fn non_bool_is_error_is_false() {
        assert_eq!(
            result_is_error(r#"{"type":"result","is_error":"yes"}"#),
            Some(false)
        );
    }

    #[test]
    fn error_result_is_error() {
        assert_eq!(
            result_is_error(r#"{"type":"result","is_error":true}"#),
            Some(true)
        );
    }

    #[test]
    fn nested_tool_use_input_is_not_a_result() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"x","input":{"type":"result"}}]}}"#;
        assert_eq!(result_is_error(line), None);
    }

    #[test]
    fn tool_result_text_is_not_a_result() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"{\"type\":\"result\"}"}]}}"#;
        assert_eq!(result_is_error(line), None);
    }

    #[test]
    fn other_types_are_not_results() {
        assert_eq!(
            result_is_error(r#"{"type":"system","subtype":"init"}"#),
            None
        );
    }

    #[test]
    fn invalid_or_empty_is_none() {
        assert_eq!(result_is_error("not json"), None);
        assert_eq!(result_is_error(""), None);
        assert_eq!(result_is_error("{"), None);
    }

    #[test]
    fn trailing_whitespace_is_tolerated() {
        assert_eq!(
            result_is_error("{\"type\":\"result\",\"is_error\":true}\r"),
            Some(true)
        );
        assert_eq!(result_is_error("  {\"type\":\"result\"}   "), Some(false));
    }

    #[test]
    fn junk_prefix_is_tolerated() {
        assert_eq!(
            result_is_error(r#"warn: {"type":"result","is_error":true}"#),
            Some(true)
        );
    }

    #[test]
    fn trailing_junk_is_tolerated() {
        assert_eq!(
            result_is_error(r#"{"type":"result","is_error":true} garbage"#),
            Some(true)
        );
    }

    #[test]
    fn type_need_not_be_first_key() {
        assert_eq!(
            result_is_error(r#"{"is_error":true,"subtype":"x","type":"result"}"#),
            Some(true)
        );
    }
}
